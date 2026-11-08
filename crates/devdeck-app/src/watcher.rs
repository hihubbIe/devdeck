//! Rediscovery (R6.1, R10.1): new projects, changed manifests and config edits reach
//! the workspaces without a restart.
//!
//! - One watcher (`notify` with `notify-debouncer-full`) for the whole app: every config
//!   workspace's `Services::watch` folders, non-recursively, and the config folder and its
//!   `workspaces/` folder. Access events are dropped (reading a file is not a change).
//! - Changes are grouped ([`PAUSE`]). A change that can change what's found rescans that
//!   workspace (in the background): a file discovery reads, a folder created or
//!   renamed, an install's programs (`toolchains::concerns`), a watched folder removed.
//!   Logs and build files written next to manifests don't. A changed workspace file reloads the configs and rescans
//!   that workspace with its new config; `machine.toml` reloads the environment (unless
//!   it can't be read: the current one stays, and a toast says why). The config files'
//!   problems are read again each time. Adding or removing a workspace file needs a
//!   restart: a toast says so, once per change of the set of files.
//! - After a rescan the watched folders change (`Services::watch`): the watcher follows.
//!   When it starts watching folders that were already there (an install's
//!   `node_modules/.bin`), what changed in them before is unseen: that workspace is
//!   rescanned once more.
//! - A workspace's kubeconfigs folder (`[cluster]`) is watched on its own route: a change
//!   there reloads that cluster's kubeconfigs (envs whose target changed reconnect), not
//!   the apps. A folder missing at start is tried again whenever the watched folders are
//!   looked at again (after any rescan); starting to watch it reloads too.
//! - The git dirs of the repositories views hold (their `HEAD`, `index`, refs…) are watched
//!   on a route of their own: a change there tells the repository (read again if a view
//!   looks, else later), never a rescan.
//! - The watcher's thread hands paths to the UI thread over a channel; nothing blocks it.

use crate::services::{GitWatches, Rescanned, Services};
use crate::shell::Shell;
use devdeck_core::config::{MACHINE_FILE, WORKSPACES_DIR, load_machine, load_workspaces};
use std::ffi::OsStr;
use devdeck_ui::{ToastKind, toasts};
use gpui::{App, Entity, Subscription, Task};
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
    _rescans: Vec<Subscription>,
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
        Ok(d) => d,
        Err(e) => {
            let text = format!("Folders aren't watched ({e}): restart Kugelblitz to see new projects.");
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
            return None;
        }
    };
    let watching = Rc::new(RefCell::new(Watching { debouncer, watched: BTreeSet::new(), warned: false }));
    follow(shell, &config_dir, &watching, cx);
    let files = RefCell::new(workspace_files(&config_dir));
    let workspaces: Vec<Entity<Services>> = shell.read(cx).workspaces.iter().map(|w| w.services.clone()).collect();
    let mut rescans: Vec<Subscription> = workspaces
        .iter()
        .map(|s| {
            let (shell, config_dir, watching) = (shell.clone(), config_dir.clone(), watching.clone());
            cx.subscribe(s, move |services, _: &Rescanned, cx| {
                if follow(&shell, &config_dir, &watching, cx) {
                    services.update(cx, |s, cx| s.rescan(None, cx));
                }
            })
        })
        .collect();
    rescans.extend(workspaces.iter().map(|s| {
        let (shell, config_dir, watching) = (shell.clone(), config_dir.clone(), watching.clone());
        cx.subscribe(s, move |_, _: &GitWatches, cx| {
            follow(&shell, &config_dir, &watching, cx);
        })
    }));
    let shell = shell.clone();
    let task = cx.spawn(async move |cx| {
        while let Ok(mut paths) = rx.recv().await {
            while let Ok(more) = rx.try_recv() {
                paths.extend(more);
            }
            cx.update(|cx| changed(&shell, &config_dir, &files, &paths, cx));
        }
    });
    Some(Watcher { _task: task, _rescans: rescans })
}

/// Keeps a [`Watcher`] for the app's lifetime.
pub fn keep(watcher: Watcher, cx: &mut App) {
    struct Kept(#[allow(dead_code)] Watcher);
    impl gpui::Global for Kept {}
    cx.set_global(Kept(watcher));
}

struct Watching {
    debouncer: Inner,
    watched: BTreeSet<PathBuf>,
    /// A folder couldn't be watched, and that was said.
    warned: bool,
}

/// Watches what should be watched now, and stops watching what shouldn't. The first
/// folder that can't be watched (other than a missing one) is reported, once. Whether
/// it started watching a folder of projects (git dirs don't count: no rescan for them).
fn follow(shell: &Entity<Shell>, config_dir: &Path, watching: &RefCell<Watching>, cx: &mut App) -> bool {
    let mut wanted: BTreeSet<PathBuf> = [config_dir.to_path_buf(), config_dir.join(WORKSPACES_DIR)].into();
    let mut kubeconfigs: Vec<(PathBuf, Entity<Services>)> = Vec::new();
    let (mut git, mut recursive) = (BTreeSet::new(), BTreeSet::new());
    for w in &shell.read(cx).workspaces {
        let s = w.services.read(cx);
        wanted.extend(s.watch.iter().cloned());
        if let Some(k) = s.kubeconfigs() {
            wanted.insert(k.to_path_buf());
            kubeconfigs.push((k.to_path_buf(), w.services.clone()));
        }
        for (dir, deep) in s.git_watch(cx) {
            if deep {
                recursive.insert(dir.clone());
            }
            git.insert(dir.clone());
            wanted.insert(dir);
        }
    }
    let (mut failed, mut started) = (None, false);
    let mut reload = Vec::new();
    {
        let w = &mut *watching.borrow_mut();
        for gone in w.watched.difference(&wanted).cloned().collect::<Vec<_>>() {
            let _ = w.debouncer.unwatch(&gone);
            w.watched.remove(&gone);
        }
        // A folder that doesn't exist yet (no `workspaces/`) is tried again next time.
        for new in wanted.difference(&w.watched.clone()) {
            let mode = if recursive.contains(new) { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
            match w.debouncer.watch(new, mode) {
                Ok(()) => {
                    w.watched.insert(new.clone());
                    started |= !git.contains(new);
                    reload.extend(kubeconfigs.iter().filter(|(k, _)| k == new).map(|(_, s)| s.clone()));
                }
                Err(e) if new.exists() && !w.warned => {
                    w.warned = true;
                    failed = Some(format!("{} isn't watched ({e}): new projects there show after a restart.", new.display()));
                }
                Err(_) => {}
            }
        }
    }
    if let Some(text) = failed {
        toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
    }
    for s in reload {
        reload_cluster(&s, cx);
    }
    started
}

fn reload_cluster(s: &Entity<Services>, cx: &mut App) {
    if let Some(c) = s.read(cx).cluster.clone() {
        c.update(cx, |c, cx| c.reload(cx));
    }
}

/// Reloads the clusters whose kubeconfigs folder `paths` touch.
fn reload_kubeconfigs(shell: &Entity<Shell>, paths: &[PathBuf], cx: &mut App) {
    let workspaces: Vec<_> = shell.read(cx).workspaces.iter().map(|w| w.services.clone()).collect();
    for s in workspaces {
        let touched = s.read(cx).kubeconfigs().is_some_and(|k| paths.iter().any(|p| p == k || p.parent() == Some(k)));
        if touched {
            reload_cluster(&s, cx);
        }
    }
}

/// Rescans what `paths` concern. `files`: the workspace files there were.
fn changed(shell: &Entity<Shell>, config_dir: &Path, files: &RefCell<BTreeSet<String>>, paths: &[PathBuf], cx: &mut App) {
    let workspaces_dir = config_dir.join(WORKSPACES_DIR);
    let machine_changed = paths.iter().any(|p| *p == config_dir.join(MACHINE_FILE));
    let workspaces_changed = paths.iter().any(|p| p.parent() == Some(workspaces_dir.as_path()));
    reload_kubeconfigs(shell, paths, cx);
    let workspaces: Vec<_> = shell.read(cx).workspaces.iter().map(|w| w.services.clone()).collect();
    for s in workspaces {
        s.update(cx, |s, cx| s.git_changed(paths, cx));
    }
    if !machine_changed && !workspaces_changed {
        return rescan_concerned(shell, paths, cx);
    }
    let (configs, errors) = load_workspaces(config_dir);
    let (machine, machine_errors) = load_machine(config_dir);
    let problems: Vec<String> = errors.iter().chain(&machine_errors).map(ToString::to_string).collect();
    let env = crate::environment::environment(cx);
    env.update(cx, |e, cx| {
        if e.config_problems != problems {
            e.config_problems = problems;
            cx.notify();
        }
    });
    if machine_changed {
        if machine_errors.is_empty() {
            crate::environment::load(machine, cx);
        } else {
            for e in &machine_errors {
                toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("{e}; keeping the previous machine settings"), vec![], cx));
            }
        }
    }
    if workspaces_changed {
        let now = workspace_files(config_dir);
        if *files.borrow() != now {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Info, "Restart Kugelblitz to add or remove a workspace.", vec![], cx));
            *files.borrow_mut() = now;
        }
        for e in &errors {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, e.to_string(), vec![], cx));
        }
        let workspaces: Vec<_> = shell.read(cx).workspaces.iter().map(|w| w.services.clone()).collect();
        for config in configs {
            let same = |s: &&Entity<crate::services::Services>| s.read(cx).config.as_ref().is_some_and(|c| c.id == config.id);
            if let Some(s) = workspaces.iter().find(same).cloned()
                && s.read(cx).config.as_ref() != Some(&config)
            {
                s.update(cx, |s, cx| s.rescan(Some(config), cx));
            }
        }
    }
    rescan_concerned(shell, paths, cx);
}

/// Rescans the workspaces whose folders `paths` can change.
fn rescan_concerned(shell: &Entity<Shell>, paths: &[PathBuf], cx: &mut App) {
    let workspaces: Vec<_> = shell.read(cx).workspaces.iter().map(|w| w.services.clone()).collect();
    for s in workspaces {
        let concerned = {
            let s = s.read(cx);
            let watched = |p: &Path| s.watch.iter().any(|w| w == p);
            s.config.is_some() && paths.iter().any(|p| watched(p) || (p.parent().is_some_and(watched) && devdeck_core::toolchains::concerns(p)))
        };
        if concerned {
            s.update(cx, |s, cx| s.rescan(None, cx));
        }
    }
}

/// The workspace files in the config folder, readable or not (`work` for `work.toml`;
/// hidden ones are editors', as for `load_workspaces`).
fn workspace_files(config_dir: &Path) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(config_dir.join(WORKSPACES_DIR)) else { return BTreeSet::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension() == Some(OsStr::new("toml")))
        .filter_map(|p| Some(p.file_stem()?.to_str()?.to_string()))
        .filter(|stem| !stem.starts_with('.'))
        .collect()
}
