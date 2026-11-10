//! Ports of real runs (R6.9).
//!
//! - A background scan (`core::ports::listening` over the trees of `Runner::pids`) every
//!   ~2 s, only while a run with a process runs or a port view is shown; it reads `/proc`
//!   off the UI thread, results are cached here and applied on the UI thread, and a
//!   change notifies. Nothing is computed during render (R1.3).
//! - Detected ports are remembered per command (`core::ports::PortMemory`, in the state
//!   folder, loaded when first needed); an empty scan never erases them.
//! - Before a start, a remembered or declared port that someone holds refuses the start
//!   with "port 5720 is held by …"; the toast offers "Stop holder": our own run stops at
//!   once; another process is stopped (SIGTERM) only after a confirmation that names it.
//! - `Services::ports` reads this cache (plus the demo's fake holders).
//!
//! The demo has no machine scan, no memory and no conflicts: its runs are fake.

use crate::runner::{RunKey, RunRefusal, Runner};
use devdeck_core::ports::{Holder, PortMemory, holder, listening};
use devdeck_core::procfs;
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{ModalFocus, ToastKind, button, label, modal_frame, pop_modal, push_modal, theme, toasts};
use gpui::{
    App, AppContext, Context, Entity, EntityId, FocusHandle, IntoElement, ParentElement, Render, Styled, Subscription, Task, WeakEntity,
    WeakFocusHandle, Window, actions, div, prelude::*,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

actions!(port_scan, [ConfirmStopHolder]);

/// Time between two scans.
pub const SCAN_EVERY: Duration = Duration::from_secs(2);

/// What the last scan found, per run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortScan {
    pub listening: BTreeMap<RunKey, Vec<u16>>,
    /// Who holds the ports port views show, whoever it is.
    pub held: BTreeMap<u16, PortHolder>,
}

/// Who holds a port, as the UI acts on it.
#[derive(Clone, Debug, PartialEq)]
pub enum PortHolder {
    /// One of this workspace's runs (or a process below it): stopped at once.
    Run { key: RunKey, title: String },
    /// Another process: stopped only after a confirmation naming it.
    Process { pid: u32, command: String },
    /// Listening, but the process isn't readable (another user): can't be stopped here.
    Unknown,
}

impl PortHolder {
    pub fn stoppable(&self) -> bool {
        !matches!(self, PortHolder::Unknown)
    }
}

impl fmt::Display for PortHolder {
    /// `entityservice · run`, `node (pid 7712)`, `another user's process`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortHolder::Run { title, .. } => write!(f, "{title}"),
            PortHolder::Process { pid, command } => match program(command) {
                Some(name) => write!(f, "{name} (pid {pid})"),
                None => write!(f, "pid {pid}"),
            },
            PortHolder::Unknown => write!(f, "another user's process"),
        }
    }
}

/// The program's file name in a command line (`/usr/bin/node x.js` → `node`).
pub fn program(command: &str) -> Option<&str> {
    let first = command.split_whitespace().next()?;
    first.rsplit('/').next().filter(|n| !n.is_empty())
}

/// Held by the control API while it wants ports scanned ([`Ports::interest`]).
pub struct PortsInterest(#[allow(dead_code)] std::rc::Rc<BTreeSet<u16>>);

/// The scan and port memory of one workspace.
pub struct Ports {
    workspace: String,
    runner: Entity<Runner>,
    /// `None` for the demo: no machine scan, nothing remembered, no conflicts.
    state_dir: Option<PathBuf>,
    /// Loaded from `state_dir` when first needed.
    memory: Option<PortMemory>,
    /// The ports each run listened on since it started (which start, and the ports): a
    /// scan sees only what listens at that moment, the memory keeps them all.
    seen: BTreeMap<RunKey, (u32, BTreeSet<u16>)>,
    pub scan: PortScan,
    /// Port views on screen and the port each shows.
    shown: BTreeMap<EntityId, u16>,
    /// The control API's readers and followers, with the ports they watch.
    interests: Vec<std::rc::Weak<BTreeSet<u16>>>,
    scans: u64,
    task: Option<Task<()>>,
    _observe: Subscription,
}

/// What one scan looks at, taken on the UI thread.
struct Job {
    runs: Vec<(RunKey, u32)>,
    watched: BTreeSet<u16>,
}

/// What one scan found, off the UI thread.
struct Found {
    listening: BTreeMap<RunKey, Vec<u16>>,
    held: BTreeMap<u16, (Holder, Option<RunKey>)>,
}

impl Ports {
    /// The ports of `runner`'s workspace, consulted by its starts. `state_dir` holds the
    /// port memory; `None` is the demo.
    pub fn new(workspace: &str, runner: Entity<Runner>, state_dir: Option<PathBuf>, cx: &mut App) -> Entity<Ports> {
        let ports = cx.new(|cx| Ports {
            workspace: workspace.into(),
            runner: runner.clone(),
            state_dir,
            memory: None,
            seen: BTreeMap::new(),
            scan: PortScan::default(),
            shown: BTreeMap::new(),
            interests: Vec::new(),
            scans: 0,
            task: None,
            _observe: cx.observe(&runner, |this: &mut Ports, _, cx| this.ensure(cx)),
        });
        let weak = ports.downgrade();
        runner.update(cx, |r, _| r.set_ports(weak));
        ports
    }

    /// How many scans ran.
    pub fn scans(&self) -> u64 {
        self.scans
    }

    /// Whether scans are going on.
    pub fn scanning(&self) -> bool {
        self.task.is_some()
    }

    /// A port view came on screen showing `port`.
    pub fn show(&mut self, view: EntityId, port: u16, cx: &mut Context<Self>) {
        self.shown.insert(view, port);
        self.ensure(cx);
    }

    /// A port view left the screen (or closed).
    pub fn hide(&mut self, view: EntityId) {
        self.shown.remove(&view);
    }

    /// The ports `key` listened on in an earlier run.
    pub fn remembered(&mut self, key: &RunKey) -> Vec<u16> {
        let ws = self.workspace.clone();
        self.memory().map(|m| m.get(&ws, &key.app, &key.cmd).to_vec()).unwrap_or_default()
    }

    fn memory(&mut self) -> Option<&mut PortMemory> {
        if self.memory.is_none() {
            self.memory = Some(PortMemory::load(self.state_dir.as_ref()?));
        }
        self.memory.as_mut()
    }

    /// The first of `declared` and the ports `key` used before that someone holds now.
    /// Reads `/proc`: on a start, not per frame.
    pub fn conflict(&mut self, key: &RunKey, declared: &[u16], runner: &Runner) -> Option<(u16, PortHolder)> {
        self.state_dir.as_ref()?;
        // A terminal runs whatever is typed in it: no ports of its own to check.
        if key.is_terminal() {
            return None;
        }
        let ports: BTreeSet<u16> = declared.iter().copied().chain(self.remembered(key)).collect();
        if ports.is_empty() {
            return None;
        }
        let runs = runner.pids();
        ports.into_iter().find_map(|port| {
            let h = holder(port)?;
            let owner = match &h {
                Holder::Process { pid, .. } => owner(*pid, &runs, None),
                Holder::Unknown => None,
            };
            Some((port, resolve(h, owner, runner)))
        })
    }

    fn wanted(&self, cx: &App) -> bool {
        self.state_dir.is_some() && (!self.watched().is_empty() || !self.runner.read(cx).pids().is_empty())
    }

    /// The ports port views show and the API's interests watch.
    fn watched(&self) -> BTreeSet<u16> {
        let api = self.interests.iter().filter_map(std::rc::Weak::upgrade).flat_map(|p| p.iter().copied().collect::<Vec<_>>());
        self.shown.values().copied().chain(api).collect()
    }

    /// Scans while the handle lives (the control API's reads and followers), `ports`
    /// watched with them: only while there's something to scan (a run, a watched port).
    pub fn interest(&mut self, ports: BTreeSet<u16>, cx: &mut Context<Self>) -> PortsInterest {
        self.interests.retain(|w| w.strong_count() > 0);
        let held = std::rc::Rc::new(ports);
        self.interests.push(std::rc::Rc::downgrade(&held));
        self.ensure(cx);
        PortsInterest(held)
    }

    /// Starts scanning when there is something to scan; scanning ends by itself once
    /// there isn't.
    fn ensure(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() || !self.wanted(cx) {
            return;
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok(Some(job)) = this.update(cx, |p, cx| p.job(cx)) else { return };
                let found = cx.background_spawn(async move { job.run() }).await;
                if this.update(cx, |p, cx| p.apply(found, cx)).is_err() {
                    return;
                }
                cx.background_executor().timer(SCAN_EVERY).await;
            }
        }));
    }

    /// The next scan's work, or `None` (and scanning ends) when nothing is left to scan.
    fn job(&mut self, cx: &mut Context<Self>) -> Option<Job> {
        if !self.wanted(cx) {
            self.task = None;
            if self.scan != PortScan::default() {
                self.scan = PortScan::default();
                cx.notify();
            }
            return None;
        }
        self.interests.retain(|w| w.strong_count() > 0);
        Some(Job { runs: self.runner.read(cx).pids(), watched: self.watched() })
    }

    fn apply(&mut self, found: Found, cx: &mut Context<Self>) {
        self.scans += 1;
        let ws = self.workspace.clone();
        let runner = self.runner.read(cx);
        // A terminal's ports are what it runs now, not its own to remember.
        for (key, ports) in found.listening.iter().filter(|(k, _)| !k.is_terminal()) {
            let start = runner.run(key).map_or(0, |r| r.runs);
            let seen = self.seen.entry(key.clone()).or_default();
            if seen.0 != start {
                *seen = (start, BTreeSet::new());
            }
            seen.1.extend(ports);
        }
        let seen: Vec<(RunKey, Vec<u16>)> = found.listening.keys().filter_map(|k| Some((k.clone(), self.seen.get(k)?.1.iter().copied().collect()))).collect();
        if let Some(memory) = self.memory() {
            for (key, ports) in &seen {
                // Best effort: a memory that can't be saved only loses the warning.
                let _ = memory.remember(&ws, &key.app, &key.cmd, ports);
            }
        }
        let runner = self.runner.read(cx);
        let held = found.held.into_iter().map(|(port, (h, owner))| (port, resolve(h, owner, runner))).collect();
        let scan = PortScan { listening: found.listening, held };
        if scan != self.scan {
            self.scan = scan;
            cx.notify();
        }
    }
}

impl Job {
    /// Reads `/proc`: a run's ports are those of every process below it (a dev server is
    /// often a child of the shell devdeck started).
    fn run(self) -> Found {
        let pairs = if self.runs.is_empty() { Vec::new() } else { procfs::parents() };
        let trees: Vec<(RunKey, BTreeSet<u32>)> = self.runs.iter().map(|(k, pid)| (k.clone(), procfs::tree(*pid, &pairs))).collect();
        let by_pid: BTreeMap<u32, &RunKey> = trees.iter().flat_map(|(k, pids)| pids.iter().map(move |p| (*p, k))).collect();
        let mut found: BTreeMap<RunKey, Vec<u16>> = BTreeMap::new();
        if !by_pid.is_empty() {
            let pids: Vec<u32> = by_pid.keys().copied().collect();
            for l in listening(&pids) {
                if let Some(key) = by_pid.get(&l.pid) {
                    found.entry((*key).clone()).or_default().push(l.port);
                }
            }
        }
        for ports in found.values_mut() {
            ports.sort_unstable();
            ports.dedup();
        }
        let held = self
            .watched
            .iter()
            .filter_map(|&port| {
                let h = holder(port)?;
                let owner = match &h {
                    Holder::Process { pid, .. } => owner(*pid, &self.runs, Some(&by_pid)),
                    Holder::Unknown => None,
                };
                Some((port, (h, owner)))
            })
            .collect();
        Found { listening: found, held }
    }
}

/// The run `pid` belongs to: a run's process or a process below one.
fn owner(pid: u32, runs: &[(RunKey, u32)], trees: Option<&BTreeMap<u32, &RunKey>>) -> Option<RunKey> {
    if let Some(key) = trees.and_then(|t| t.get(&pid)) {
        return Some((*key).clone());
    }
    // Up through the parents, as far as a run (a few levels at most in practice).
    let mut cur = pid;
    for _ in 0..64 {
        if let Some((key, _)) = runs.iter().find(|(_, p)| *p == cur) {
            return Some(key.clone());
        }
        cur = procfs::parent_of(cur).filter(|p| *p > 1)?;
    }
    None
}

fn resolve(h: Holder, owner: Option<RunKey>, runner: &Runner) -> PortHolder {
    match (h, owner) {
        (_, Some(key)) => PortHolder::Run { title: runner.title(&key), key },
        (Holder::Process { pid, command }, None) => PortHolder::Process { pid, command },
        (Holder::Unknown, None) => PortHolder::Unknown,
    }
}

// ------------------------------------------------------------------ starts and stops

/// Starts `key` with `env`; a refusal becomes a warning toast (see [`refusal_toast`]).
pub fn start_or_toast(runner: &Entity<Runner>, key: &RunKey, env: Option<String>, cx: &mut App) {
    if let Err(why) = runner.update(cx, |r, cx| r.try_start(key, env, cx)) {
        refusal_toast(runner, key, &why, cx);
    }
}

/// "app · command: why", with "Stop holder" when a port is held by something that can
/// be stopped.
pub fn refusal_toast(runner: &Entity<Runner>, key: &RunKey, why: &RunRefusal, cx: &mut App) {
    let text = format!("{}: {why}", runner.read(cx).title(key));
    let mut actions: Vec<(gpui::SharedString, devdeck_ui::overlay::MenuAction)> = Vec::new();
    // A process devdeck didn't start is stopped only in edit mode (R5.6): not offered then.
    if let RunRefusal::PortHeld { port, holder } = why
        && holder.stoppable()
        && (matches!(holder, PortHolder::Run { .. }) || crate::edit_mode::is_on(cx))
    {
        let (runner, port, holder) = (runner.downgrade(), *port, holder.clone());
        actions.push(("Stop holder".into(), Rc::new(move |window, cx| stop_holder(&runner, port, &holder, window, cx))));
    }
    toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, actions, cx));
}

/// Stops what holds `port`: one of our runs at once; another process after a
/// confirmation naming it.
pub fn stop_holder(runner: &WeakEntity<Runner>, port: u16, holder: &PortHolder, window: &mut Window, cx: &mut App) {
    match holder {
        PortHolder::Run { key, .. } => {
            let _ = runner.update(cx, |r, cx| r.stop(key, cx));
        }
        // Not one of devdeck's runs: only in edit mode (R5.6).
        PortHolder::Process { .. } if crate::edit_mode::refusal(cx).is_some() => {
            let why = crate::edit_mode::OFF;
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("Stop holder: {why}."), vec![], cx));
        }
        PortHolder::Process { pid, command } => confirm_stop(port, *pid, command.clone(), window, cx),
        PortHolder::Unknown => {}
    }
}

fn confirm_stop(port: u16, pid: u32, command: String, window: &mut Window, cx: &mut App) {
    let restore = window.focused(cx).map(|f| f.downgrade());
    let modal = cx.new(|cx| StopProcess { port, pid, command, focus: ModalFocus(vec![cx.focus_handle(), cx.focus_handle()]), restore });
    push_modal(modal.clone().into(), window, cx);
    let first = modal.read(cx).focus.0[0].clone();
    first.focus(window, cx);
}

/// "Stop node (pid 7712)?" with its command line; Enter stops, Esc cancels.
pub struct StopProcess {
    pub port: u16,
    pub pid: u32,
    pub command: String,
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
}

impl StopProcess {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
        // The pid may have ended and been reused since the question: it must still hold
        // the port.
        let still = matches!(holder(self.port), Some(Holder::Process { pid, .. }) if pid == self.pid);
        // Edit mode may have been turned off while this asked.
        let result = if let Some(why) = crate::edit_mode::refusal(cx) {
            Err(why)
        } else if still { devdeck_core::process::terminate(self.pid) } else { Err(format!("pid {} no longer holds port {}", self.pid, self.port)) };
        if let Err(why) = result {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, why, vec![], cx));
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.0[1].is_focused(window) {
            self.close(window, cx);
        } else {
            self.stop(window, cx);
        }
    }
}

fn focus_button(id: &'static str, text: &'static str, handle: &FocusHandle, t: &devdeck_ui::Theme) -> gpui::Stateful<gpui::Div> {
    let ring = t.focus;
    button(id, text, t).border_1().border_color(t.line).track_focus(handle).focus(move |s| s.border_color(ring))
}

impl Render for StopProcess {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let name = PortHolder::Process { pid: self.pid, command: self.command.clone() }.to_string();
        let content = div()
            .key_context("Modal StopHolder")
            .flex()
            .flex_col()
            .gap_2()
            .max_w(t.size(760.))
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &ConfirmStopHolder, window, cx| this.confirm(window, cx)))
            .child(div().text_color(t.hi).child(format!("Stop {name}?")))
            .child(label(format!("It holds port {} and wasn't started by Kugelblitz. It gets SIGTERM.", self.port)).text_color(t.mute))
            .child(div().px_2().py_1().rounded(t.radius_s).bg(t.bg).text_color(t.fg).debug_selector(|| "stop-holder-command".into()).child(self.command.clone()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(focus_button("stop-holder-cancel", "Cancel", &self.focus.0[1], &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))))
                    .child(
                        focus_button("stop-holder-confirm", "Stop", &self.focus.0[0], &t)
                            .debug_selector(|| "stop-holder-confirm".into())
                            .on_click(cx.listener(|this, _, w, cx| this.stop(w, cx))),
                    ),
            );
        modal_frame(content.into_any_element(), window, &t)
    }
}
