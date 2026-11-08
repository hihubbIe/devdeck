//! Lifecycle of real processes (R8.1, R8.2).
//!
//! - One devdeck at a time: a lock in the state folder; a second devdeck says one is
//!   already running and exits (and so never mistakes the first one's runs for
//!   leftovers). The demo has its own state folder, so its own lock.
//! - At start, what a previous devdeck left running (`GroupLog::leftovers`: its runs'
//!   scopes still holding processes, or groups) is offered in a dialog, one row per run:
//!   "N processes left by a previous Kugelblitz: Kill / Leave". Leave forgets them, so this
//!   devdeck doesn't kill them when it quits.
//! - SIGTERM, SIGHUP and SIGINT to devdeck: the app marks what runs as stopped to quit,
//!   kills everything, saves and quits; so the runs start again when devdeck reopens. If
//!   the app doesn't answer within [`SIGNAL_WAIT`], the signal thread kills every
//!   recorded live group (`process::kill_all(GroupLog::live())`) and what this devdeck's
//!   runs left in their scopes (`GroupLog::end_ours`) itself.
//! - Quitting with runs waits for their stop sequence, then kills what's left, the
//!   runs' scopes included (`Shell::stop_all_and_quit`, `Shell::kill_everything`).

use crate::shell::{Shell, focus_button};
use devdeck_core::process::{GroupLog, Leftover, kill_all};
use std::time::Duration;
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{ModalFocus, dot, label, modal_frame, pop_modal, push_modal, theme};
use gpui::{App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, actions, div, prelude::*};
use std::io::{Read, Seek, Write};
use std::path::Path;

actions!(lifecycle, [KillLeftovers]);

/// How long the signal thread waits for the app to kill and save before killing itself.
pub const SIGNAL_WAIT: Duration = Duration::from_secs(1);

/// Held while this devdeck runs; the lock goes with the process.
pub struct InstanceLock {
    _file: std::fs::File,
}

/// Claims the state folder for this devdeck, or says which devdeck has it.
pub fn claim(state_dir: &Path) -> Result<InstanceLock, String> {
    use rustix::fs::{FlockOperation, flock};
    std::fs::create_dir_all(state_dir).map_err(|e| format!("{}: {e}", state_dir.display()))?;
    let path = state_dir.join("devdeck.lock");
    let mut file =
        std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if flock(&file, FlockOperation::NonBlockingLockExclusive).is_err() {
        let mut pid = String::new();
        let _ = file.read_to_string(&mut pid);
        return Err(format!("Kugelblitz is already running (pid {})", pid.trim()));
    }
    // Who holds it, for the message a second devdeck shows.
    let _ = file.set_len(0).and_then(|_| file.rewind()).and_then(|_| write!(file, "{}", std::process::id()));
    Ok(InstanceLock { _file: file })
}

/// Once the shell is up: the leftovers dialog, and stopping everything on a signal.
pub fn install(shell: &Entity<Shell>, log: GroupLog, cx: &mut App) {
    shell.update(cx, |s, _| s.log = Some(log.clone()));
    offer_leftovers(shell, &log, cx);
    watch_signals(shell, log, cx);
}

/// The leftovers dialog in the main window, when a previous devdeck left processes.
pub fn offer_leftovers(shell: &Entity<Shell>, log: &GroupLog, cx: &mut App) {
    let Some(main) = shell.read(cx).main_window() else { return };
    let log = log.clone();
    // Reads `/proc` and the cgroup tree: not on the UI thread.
    cx.spawn(async move |cx| {
        let found = log.clone();
        let leftovers = cx.background_spawn(async move { found.leftovers() }).await;
        if !leftovers.is_empty() {
            let _ = main.update(cx, |_, window, cx| open_leftovers(leftovers, log, window, cx));
        }
    })
    .detach();
}

/// SIGTERM, SIGHUP or SIGINT: the app marks what runs as stopped to quit (before any
/// exit of a killed run reaches it), kills everything, saves and quits. The signal thread
/// then kills every recorded group anyway (it reads `/proc`, so not in a signal handler):
/// once the app answered, or after [`SIGNAL_WAIT`] if it's stuck.
fn watch_signals(shell: &Entity<Shell>, log: GroupLog, cx: &mut App) {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    let Ok(mut signals) = signal_hook::iterator::Signals::new([SIGTERM, SIGHUP, SIGINT]) else { return };
    let (tx, rx) = async_channel::bounded::<()>(1);
    let (done_tx, done_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let spawned = std::thread::Builder::new().name("devdeck signals".into()).spawn(move || {
        if signals.forever().next().is_some() {
            let _ = tx.send_blocking(());
            let _ = done_rx.recv_timeout(SIGNAL_WAIT);
            kill_all(&log.live());
            log.end_ours();
        }
    });
    if spawned.is_err() {
        return;
    }
    let shell = shell.downgrade();
    cx.spawn(async move |cx| {
        if rx.recv().await.is_ok() {
            cx.update(|cx| {
                if let Some(s) = shell.upgrade() {
                    s.update(cx, |s, cx| s.kill_to_quit(cx));
                }
                let _ = done_tx.try_send(());
                cx.quit();
            });
        }
    })
    .detach();
}

fn open_leftovers(leftovers: Vec<Leftover>, log: GroupLog, window: &mut Window, cx: &mut App) {
    let dialog = cx.new(|cx| Leftovers::new(leftovers, log, cx));
    push_modal(dialog.clone().into(), window, cx);
    let first = dialog.read(cx).focus.0[0].clone();
    first.focus(window, cx);
}

/// "N processes left by a previous Kugelblitz", one row per run (its name and what runs
/// in it, or a group's command): Kill (Enter) or Leave (Esc).
pub struct Leftovers {
    pub leftovers: Vec<Leftover>,
    /// Each one's row.
    pub rows: Vec<String>,
    pub focus: ModalFocus,
    log: GroupLog,
}

impl Leftovers {
    fn new(leftovers: Vec<Leftover>, log: GroupLog, cx: &mut Context<Self>) -> Self {
        let rows = leftovers.iter().map(row).collect();
        Self { leftovers, rows, focus: ModalFocus(vec![cx.focus_handle(), cx.focus_handle()]), log }
    }

    /// Kills the ones still the processes offered (the dialog may have stayed open).
    fn kill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.log.kill(&self.leftovers);
        pop_modal(window, cx);
    }

    /// Leaves them running, for good: quitting this devdeck won't kill them.
    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.log.forget(&self.leftovers);
        pop_modal(window, cx);
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.0[1].is_focused(window) { self.leave(window, cx) } else { self.kill(window, cx) }
    }
}

impl Render for Leftovers {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let n: usize = self.leftovers.iter().map(|l| l.pids.len()).sum();
        let title = if n == 1 { "1 process left by a previous Kugelblitz".to_string() } else { format!("{n} processes left by a previous Kugelblitz") };
        let list = div().flex().flex_col().gap_1().children(
            self.rows.iter().map(|r| div().flex().items_center().gap_1().child(dot(t.warn, true, &t)).child(label(r.clone()).text_color(t.warn))),
        );
        let buttons = div()
            .flex()
            .justify_end()
            .gap_2()
            .child(focus_button("leftovers-leave", "Leave", &self.focus.0[1], &t).on_click(cx.listener(|this, _, w, cx| this.leave(w, cx))))
            .child(
                focus_button("leftovers-kill", "Kill", &self.focus.0[0], &t)
                    .debug_selector(|| "leftovers-kill".into())
                    .on_click(cx.listener(|this, _, w, cx| this.kill(w, cx))),
            );
        let content = div()
            .key_context("Modal Leftovers")
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.leave(window, cx)))
            .on_action(cx.listener(|this, _: &KillLeftovers, window, cx| this.confirm(window, cx)))
            .child(div().text_color(t.hi).child(title))
            .child(label("Kugelblitz didn't stop them (it was killed or crashed). They may hold ports.").text_color(t.mute))
            .child(list)
            .child(buttons);
        modal_frame(content.into_any_element(), window, &t)
    }
}

/// A leftover's row: a run's name and the commands in its scope (`web · dev: vite,
/// esbuild`), or a group's command and pid.
fn row(l: &Leftover) -> String {
    match &l.label {
        Some(name) => {
            let mut commands: Vec<String> = l.pids.iter().map(|p| command_of(*p)).filter(|c| !c.is_empty()).collect();
            commands.dedup();
            format!("{name}: {}", commands.join(", "))
        }
        None => l.pids.iter().map(|p| format!("{} (pid {p})", command_of(*p))).collect::<Vec<_>>().join(", "),
    }
}

/// A process's command line, as `ps` shows it.
fn command_of(pid: u32) -> String {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| b.split(|c| *c == 0).filter(|s| !s.is_empty()).map(String::from_utf8_lossy).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}
