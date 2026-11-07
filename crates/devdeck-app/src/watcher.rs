//! Rediscovery (R6.1, R10.1): new projects, changed manifests and config edits reach
//! the workspaces without a restart.
//!
//! - One watcher (`notify` with `notify-debouncer-full`) for the whole app: every config
//!   workspace's `Services::watch` folders, non-recursively, and the config folder and its
//!   `workspaces/` folder. Access events are dropped (reading a file is not a change).
//! - Changes are grouped ([`PAUSE`]). A change under a workspace's folders rescans that
//!   workspace; a changed workspace file reloads the configs and rescans that workspace
//!   with its new config; `machine.toml` reloads the environment. Adding or removing a
//!   workspace file needs a restart: a toast says so.
//! - After a rescan the watched folders change (`Services::watch`): the watcher follows.
//! - The watcher's thread hands paths to the UI thread over a channel; nothing blocks it.

use crate::shell::Shell;
use devdeck_core::config::{ConfigError, MACHINE_FILE, WORKSPACES_DIR, load_machine, load_workspaces};
use devdeck_ui::{ToastKind, toasts};
use gpui::{App, Entity, Task};
use notify_debouncer_full::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

/// Changes closer together than this are handled together.
pub const PAUSE: Duration = Duration::from_millis(300);

type Inner = Debouncer<RecommendedWatcher, RecommendedCache>;

/// Keeps the watching alive; dropping it stops it.
pub struct Watcher {
    _task: Task<()>,
}

/// Starts watching for the shell's config workspaces and `config_dir`. When the system
/// refuses, that's reported once and devdeck works without it.
pub fn start(shell: &Entity<Shell>, config_dir: PathBuf, cx: &mut App) -> Option<Watcher> {
    let (tx, rx) = async_channel::unbounded::<Vec<PathBuf>>();
    let handler = move |result: DebounceEventResult| {
        let Ok(events) = result else { return };
        let paths: Vec<PathBuf> = events.into_iter().filter(|e| !e.kind.is_access()).flat_map(|e| e.event.paths).collect();
        if !paths.is_empty() {
            let _ = tx.send_blocking(paths);
        }
    };
    let debouncer = match new_debouncer(PAUSE, None, handler) {
        Ok(d) => Rc::new(RefCell::new(d)),
        Err(e) => {
            let text = format!("Folders aren't watched ({e}): restart devdeck to see new projects.");
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
            return None;
        }
    };
    let watched = Rc::new(RefCell::new(BTreeSet::new()));
    follow(shell, &config_dir, &debouncer, &watched, cx);
    let shell = shell.clone();
    let task = cx.spawn(async move |cx| {
        while let Ok(mut paths) = rx.recv().await {
            while let Ok(more) = rx.try_recv() {
                paths.extend(more);
            }
            cx.update(|cx| {
                changed(&shell, &config_dir, &paths, cx);
                follow(&shell, &config_dir, &debouncer, &watched, cx);
            });
        }
    });
    Some(Watcher { _task: task })
}

/// Keeps a [`Watcher`] for the app's lifetime.
pub fn keep(watcher: Watcher, cx: &mut App) {
    struct Kept(#[allow(dead_code)] Watcher);
    impl gpui::Global for Kept {}
    cx.set_global(Kept(watcher));
}

/// Watches what should be watched now, and stops watching what shouldn't.
fn follow(shell: &Entity<Shell>, config_dir: &Path, debouncer: &Rc<RefCell<Inner>>, watched: &Rc<RefCell<BTreeSet<PathBuf>>>, cx: &App) {
    let mut wanted: BTreeSet<PathBuf> = [config_dir.to_path_buf(), config_dir.join(WORKSPACES_DIR)].into();
    for w in &shell.read(cx).workspaces {
        wanted.extend(w.services.read(cx).watch.iter().cloned());
    }
    let (mut d, mut watched) = (debouncer.borrow_mut(), watched.borrow_mut());
    for gone in watched.difference(&wanted).cloned().collect::<Vec<_>>() {
        let _ = d.unwatch(&gone);
        watched.remove(&gone);
    }
    // A folder that doesn't exist yet (no `workspaces/`) is tried again next time.
    for new in wanted.difference(&watched.clone()) {
        if d.watch(new, RecursiveMode::NonRecursive).is_ok() {
            watched.insert(new.clone());
        }
    }
}

/// Rescans what `paths` concern.
fn changed(shell: &Entity<Shell>, config_dir: &Path, paths: &[PathBuf], cx: &mut App) {
    if paths.iter().any(|p| *p == config_dir.join(MACHINE_FILE)) {
        let (machine, _) = load_machine(config_dir);
        crate::environment::load(machine, cx);
    }
    let workspaces: Vec<_> = shell.read(cx).workspaces.iter().map(|w| w.services.clone()).collect();
    let workspaces_dir = config_dir.join(WORKSPACES_DIR);
    if paths.iter().any(|p| p.parent() == Some(workspaces_dir.as_path())) {
        let (configs, errors) = load_workspaces(config_dir);
        // A file that can't be read now is still a workspace (its problem is shown).
        let running: BTreeSet<String> = workspaces.iter().filter_map(|s| Some(s.read(cx).config.as_ref()?.id.clone())).collect();
        let on_disk: BTreeSet<String> = configs.iter().map(|c| c.id.clone()).chain(errors.iter().filter_map(stem)).collect();
        if on_disk != running {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Info, "Restart devdeck to add or remove a workspace.", vec![], cx));
        }
        for e in &errors {
            let text = match e.line {
                Some(line) => format!("{}:{line}: {}", e.file.display(), e.message),
                None => format!("{}: {}", e.file.display(), e.message),
            };
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
        }
        for config in configs {
            let same = |s: &&Entity<crate::services::Services>| s.read(cx).config.as_ref().is_some_and(|c| c.id == config.id);
            if let Some(s) = workspaces.iter().find(same).cloned()
                && s.read(cx).config.as_ref() != Some(&config)
            {
                s.update(cx, |s, cx| s.rescan(Some(config), cx));
            }
        }
    }
    for s in workspaces {
        let concerned = {
            let s = s.read(cx);
            s.config.is_some() && paths.iter().any(|p| s.watch.iter().any(|w| p == w || p.parent() == Some(w.as_path())))
        };
        if concerned {
            s.update(cx, |s, cx| s.rescan(None, cx));
        }
    }
}

/// The workspace id a config file is for (`work` for `workspaces/work.toml`).
fn stem(e: &ConfigError) -> Option<String> {
    e.file.file_stem()?.to_str().map(String::from)
}
