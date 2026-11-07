//! The runs of one workspace (R6.7): start, stop, restart, state, output. A
//! [`RunDriver`] does the work: [`FakeDriver`](crate::fake_driver::FakeDriver) for the
//! demo, real processes in a pseudo-terminal otherwise. A driver reports output bytes
//! and the exit; the runner keeps one terminal per run, so output is kept whether or not
//! a view shows it, and the state follows the real exit.

use devdeck_core::apps::{App, Command, Tools};
use devdeck_core::catalog::Catalog;
use devdeck_core::runs::{Exit, RunState};
use devdeck_core::terminal::{TermSize, Terminal};
use crate::port_scan::{PortHolder, Ports};
use gpui::{Context, EventEmitter, Task, WeakEntity};
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

/// Lines of history kept per run, on top of the screen. A full terminal costs about
/// 2.4 KB per line at 120 columns (docs/perf/M2.md): 2 000 lines is about 5 MB a run.
pub const SCROLLBACK: usize = 2000;
/// The terminal size of a run no view has measured yet.
pub const DEFAULT_SIZE: TermSize = TermSize { cols: 120, rows: 30 };

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunKey {
    pub app: String,
    pub cmd: String,
}

impl RunKey {
    pub fn new(app: &str, cmd: &str) -> Self {
        Self { app: app.into(), cmd: cmd.into() }
    }
}

/// One start of a run key. Drivers tag what they report with it, so a late event of an
/// earlier start is ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunId {
    pub key: RunKey,
    pub n: u32,
}

/// What a driver needs to start a run. The real driver builds the process's whole
/// environment from it: the toolbox's variables, the PATH for `app` (its Node first),
/// `vars`, `TERM` and `COLORTERM`.
#[derive(Clone, Debug)]
pub struct Launch {
    pub id: RunId,
    /// Its `dir` is where the command runs.
    pub app: App,
    pub cmd: Command,
    /// The command line with the env applied.
    pub line: String,
    /// The env binding's variables.
    pub vars: Vec<(String, String)>,
    pub env: Option<String>,
    pub size: TermSize,
}

/// A started run, as the runner drives it. Dropping it ends the run.
pub trait RunControl {
    /// Input for the process (keys, terminal replies).
    fn write(&self, bytes: &[u8]);
    fn resize(&self, size: TermSize);
    /// Asks the process to stop; its exit comes back through [`Runner::exited`].
    fn stop(&self);
    /// The process id (also its process group id), when it's a real process.
    fn pid(&self) -> Option<u32> {
        None
    }
}

pub struct Started {
    pub control: Box<dyn RunControl>,
    /// The driver's work for this run; dropped with the run.
    pub task: Task<()>,
}

/// Starts runs. The driver reports with [`Runner::output`] and [`Runner::exited`].
pub trait RunDriver {
    fn spawn(&self, launch: Launch, cx: &mut Context<Runner>) -> Result<Started, String>;
}

pub struct Run {
    pub state: RunState,
    pub terminal: Terminal,
    pub env: Option<String>,
    /// How many times it started.
    pub runs: u32,
    /// Why it couldn't start, when it couldn't.
    pub error: Option<String>,
    control: Option<Box<dyn RunControl>>,
    _task: Option<Task<()>>,
    restart_after_stop: bool,
    /// How long its stop sequence takes, from its command when it started (the command
    /// may change or disappear while it runs).
    stop: std::time::Duration,
}

/// Why a start was refused, so the UI can act on it.
#[derive(Clone, Debug, PartialEq)]
pub enum RunRefusal {
    /// It can't run (no such command, a missing tool) or it already runs.
    Cannot(String),
    /// A port it listens on (declared, or remembered from an earlier run) is held (R6.9).
    PortHeld { port: u16, holder: PortHolder },
}

impl fmt::Display for RunRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunRefusal::Cannot(why) => write!(f, "{why}"),
            RunRefusal::PortHeld { port, holder } => write!(f, "port {port} is held by {holder}"),
        }
    }
}

pub enum RunnerEvent {
    /// A run ended on its own and failed (exit code, signal, or it couldn't start).
    Failed { key: RunKey, code: i32 },
    /// New output for a run. Output is an event, not a notification, so only views
    /// showing that run redraw; state changes notify.
    Output { key: RunKey },
}

pub struct Runner {
    pub workspace: String,
    catalog: Rc<Catalog>,
    runs: BTreeMap<RunKey, Run>,
    driver: Rc<dyn RunDriver>,
    /// What [`Runner::stop_all`] stopped: it was running when devdeck quit, so its view
    /// starts it again when the workspace reopens.
    stopped_to_quit: std::collections::BTreeSet<RunKey>,
    /// The workspace's ports, checked before a start (none: no check).
    ports: Option<WeakEntity<Ports>>,
}

impl EventEmitter<RunnerEvent> for Runner {}

impl Runner {
    pub fn new(workspace: String, catalog: Rc<Catalog>, driver: Rc<dyn RunDriver>) -> Self {
        Self { workspace, catalog, runs: BTreeMap::new(), driver, stopped_to_quit: Default::default(), ports: None }
    }

    /// The ports checked before a start ([`Ports::new`] sets them).
    pub fn set_ports(&mut self, ports: WeakEntity<Ports>) {
        self.ports = Some(ports);
    }

    /// A new catalog (tools found, apps rediscovered). Runs of commands that disappeared
    /// keep going; they can't start again until their command is back.
    pub fn set_catalog(&mut self, catalog: Rc<Catalog>, cx: &mut Context<Self>) {
        self.catalog = catalog;
        cx.notify();
    }

    pub fn tools(&self) -> &Tools {
        &self.catalog.tools
    }

    pub fn run(&self, key: &RunKey) -> Option<&Run> {
        self.runs.get(key)
    }

    pub fn state(&self, key: &RunKey) -> Option<RunState> {
        self.runs.get(key).map(|r| r.state)
    }

    pub fn app_running(&self, app: &str) -> bool {
        self.runs.iter().any(|(k, r)| k.app == app && r.state.is_running())
    }

    /// What runs, as titles, alphabetical (quit guard).
    pub fn running(&self) -> Vec<String> {
        self.runs.iter().filter(|(_, r)| r.state.is_running()).map(|(k, _)| self.title(k)).collect()
    }

    /// `app · command`, with the app's display name.
    pub fn title(&self, key: &RunKey) -> String {
        match self.catalog.app(&key.app) {
            Some(app) => format!("{} · {}", app.name, devdeck_core::apps::name_of(&key.cmd)),
            None => format!("{} · {}", key.app, devdeck_core::apps::name_of(&key.cmd)),
        }
    }

    /// Running keys with the ports their command listens on.
    pub fn running_ports(&self) -> Vec<(RunKey, u16)> {
        self.runs
            .iter()
            .filter(|(_, r)| r.state == RunState::Running)
            .flat_map(|(k, _)| {
                let ports = self.command(k).map(|(_, c)| c.ports.clone()).unwrap_or_default();
                ports.into_iter().map(move |p| (k.clone(), p))
            })
            .collect()
    }

    fn command(&self, key: &RunKey) -> Option<(&App, &Command)> {
        let app = self.catalog.app(&key.app)?;
        Some((app, app.command(&key.cmd)?))
    }

    /// Why `key` can't run at all, if it can't (no such command, a missing tool).
    fn blocked(&self, key: &RunKey) -> Option<String> {
        let Some((app, cmd)) = self.command(key) else {
            return Some(format!("{} has no command {}", key.app, devdeck_core::apps::name_of(&key.cmd)));
        };
        self.catalog.tools.missing(app, cmd)
    }

    /// Why `key` can't start now, if it can't.
    pub fn refusal(&self, key: &RunKey) -> Option<String> {
        self.blocked(key).or_else(|| (self.state(key) == Some(RunState::Running)).then(|| "already running".to_string()))
    }

    /// Why `key` can't start now, held ports included. Reads the machine's sockets: on a
    /// start, not per frame (use [`Runner::refusal`] there).
    pub fn refusal_detail(&self, key: &RunKey, cx: &mut gpui::App) -> Option<RunRefusal> {
        if let Some(why) = self.refusal(key) {
            return Some(RunRefusal::Cannot(why));
        }
        // Its own process may still hold its ports: it starts once stopped, checked then.
        if self.state(key) == Some(RunState::Stopping) {
            return None;
        }
        let ports = self.ports.as_ref()?.upgrade()?;
        let declared = self.command(key).map(|(_, c)| c.ports.clone()).unwrap_or_default();
        let (port, holder) = ports.update(cx, |p, _| p.conflict(key, &declared, self))?;
        Some(RunRefusal::PortHeld { port, holder })
    }

    /// Starts a command with `env`. Starting while it stops restarts it once stopped.
    pub fn start(&mut self, key: &RunKey, env: Option<String>, cx: &mut Context<Self>) -> Result<(), String> {
        self.try_start(key, env, cx).map_err(|why| why.to_string())
    }

    /// [`Runner::start`], refused with a reason the UI can act on (a held port's holder).
    pub fn try_start(&mut self, key: &RunKey, env: Option<String>, cx: &mut Context<Self>) -> Result<(), RunRefusal> {
        if let Some(why) = self.refusal_detail(key, cx) {
            return Err(why);
        }
        if let Some(r) = self.runs.get_mut(key)
            && r.state == RunState::Stopping
        {
            r.restart_after_stop = true;
            r.env = env;
            return Ok(());
        }
        let (app, cmd) = self.command(key).map(|(a, c)| (a.clone(), c.clone())).expect("checked");
        // Each start gets a fresh terminal, at the size its view last asked for.
        let size = self.runs.get(key).map_or(DEFAULT_SIZE, |r| r.terminal.size());
        let terminal = Terminal::new(size, SCROLLBACK);
        let run = match self.runs.get_mut(key) {
            Some(run) => {
                run.terminal = terminal;
                run
            }
            None => self.runs.entry(key.clone()).or_insert(Run {
                state: RunState::Stopped,
                terminal,
                env: None,
                runs: 0,
                error: None,
                control: None,
                _task: None,
                restart_after_stop: false,
                stop: Default::default(),
            }),
        };
        run.runs += 1;
        run.state = RunState::Running;
        run.env = env.clone();
        run.error = None;
        run.restart_after_stop = false;
        run.stop = devdeck_core::process::StopKind::of(&cmd).total();
        let applied = app.env.apply(&cmd.line, env.as_deref());
        let header = format!("── run {} · {}{} ──", run.runs, applied.line, env.as_deref().map(|e| format!(" · @{e}")).unwrap_or_default());
        note(&mut run.terminal, &header);
        let id = RunId { key: key.clone(), n: run.runs };
        let launch = Launch { id: id.clone(), app, cmd, line: applied.line, vars: applied.vars, env, size };
        let driver = self.driver.clone();
        match driver.spawn(launch, cx) {
            Ok(started) => {
                if let Some(r) = self.runs.get_mut(key) {
                    r.control = Some(started.control);
                    r._task = Some(started.task);
                }
            }
            Err(why) => self.exited(&id, Exit::SpawnFailed(why), cx),
        }
        cx.notify();
        Ok(())
    }

    /// Asks a run to stop; it is Stopped once the process exits. Stopping a run that
    /// is already stopping cancels a pending restart.
    pub fn stop(&mut self, key: &RunKey, cx: &mut Context<Self>) {
        let Some(run) = self.runs.get_mut(key) else { return };
        if run.state == RunState::Stopping {
            run.restart_after_stop = false;
            cx.notify();
            return;
        }
        if run.state != RunState::Running {
            return;
        }
        run.state = RunState::Stopping;
        if let Some(c) = &run.control {
            c.stop();
        }
        cx.notify();
    }

    pub fn restart(&mut self, key: &RunKey, env: Option<String>, cx: &mut Context<Self>) -> Result<(), String> {
        match self.state(key) {
            Some(RunState::Running) => {
                if let Some(why) = self.blocked(key) {
                    return Err(why);
                }
                self.stop(key, cx);
                if let Some(r) = self.runs.get_mut(key) {
                    r.restart_after_stop = true;
                    r.env = env;
                }
                Ok(())
            }
            _ => self.start(key, env, cx),
        }
    }

    /// Remembers what runs as stopped only because devdeck quits, whatever ends it next
    /// (its stop sequence, or a kill).
    pub fn quitting(&mut self) {
        let keys = self.runs.iter().filter(|(_, r)| r.state.is_running()).map(|(k, _)| k.clone());
        self.stopped_to_quit.extend(keys);
    }

    /// Stops everything to quit, remembering what ran.
    pub fn stop_all(&mut self, cx: &mut Context<Self>) {
        self.quitting();
        let keys: Vec<RunKey> = self.runs.iter().filter(|(_, r)| r.state == RunState::Running).map(|(k, _)| k.clone()).collect();
        for k in keys {
            self.stop(&k, cx);
        }
        for r in self.runs.values_mut() {
            r.restart_after_stop = false;
        }
    }

    /// Running, or stopped only because devdeck quits: what a run view saves as "running".
    pub fn was_running(&self, key: &RunKey) -> bool {
        self.state(key).is_some_and(RunState::is_running) || self.stopped_to_quit.contains(key)
    }

    /// How long the slowest stop sequence of the real processes running now takes (zero
    /// when none run): quitting waits that long before killing what's left.
    pub fn longest_stop(&self) -> std::time::Duration {
        self.runs
            .iter()
            .filter(|(_, r)| r.state.is_running() && r.control.as_ref().is_some_and(|c| c.pid().is_some()))
            .map(|(_, r)| r.stop)
            .max()
            .unwrap_or_default()
    }

    /// The process ids of the runs that have one (port scans, quitting).
    pub fn pids(&self) -> Vec<(RunKey, u32)> {
        self.runs.iter().filter_map(|(k, r)| Some((k.clone(), r.control.as_ref()?.pid()?))).collect()
    }

    /// Input typed into a run's terminal.
    pub fn write(&mut self, key: &RunKey, bytes: &[u8]) {
        if let Some(c) = self.runs.get(key).and_then(|r| r.control.as_ref()) {
            c.write(bytes);
        }
    }

    /// The size a view shows a run's terminal at.
    pub fn resize(&mut self, key: &RunKey, size: TermSize, cx: &mut Context<Self>) {
        let Some(run) = self.runs.get_mut(key) else { return };
        if run.terminal.size() == size {
            return;
        }
        run.terminal.resize(size);
        if let Some(c) = &run.control {
            c.resize(size);
        }
        cx.emit(RunnerEvent::Output { key: key.clone() });
    }

    /// Driver: output of a run.
    pub fn output(&mut self, id: &RunId, bytes: &[u8], cx: &mut Context<Self>) {
        let Some(run) = self.runs.get_mut(&id.key) else { return };
        if run.runs != id.n || !run.state.is_running() {
            return;
        }
        run.terminal.feed(bytes);
        let replies = run.terminal.take_replies();
        if !replies.is_empty()
            && let Some(c) = &run.control
        {
            c.write(&replies);
        }
        cx.emit(RunnerEvent::Output { key: id.key.clone() });
    }

    /// Driver: the run ended (or never started).
    pub fn exited(&mut self, id: &RunId, exit: Exit, cx: &mut Context<Self>) {
        let Some(run) = self.runs.get_mut(&id.key) else { return };
        if run.runs != id.n || !run.state.is_running() {
            return;
        }
        run.control = None;
        run._task = None;
        let (state, text) = match (run.state, exit) {
            (RunState::Stopping, _) => (RunState::Stopped, "stopped".to_string()),
            (_, Exit::Code(code)) => (RunState::Exited(code), format!("exited with code {code}")),
            (_, Exit::Signal(signal)) => (RunState::Killed(signal), format!("killed by signal {signal}")),
            (_, Exit::SpawnFailed(why)) => {
                let text = format!("couldn't start: {why}");
                run.error = Some(why);
                (RunState::NotStarted, text)
            }
        };
        run.state = state;
        note(&mut run.terminal, &text);
        let again = run.restart_after_stop.then(|| run.env.clone());
        run.restart_after_stop = false;
        if let Some(code) = state.failed() {
            cx.emit(RunnerEvent::Failed { key: id.key.clone(), code });
        }
        cx.emit(RunnerEvent::Output { key: id.key.clone() });
        if let Some(env) = again
            && let Err(why) = self.start(&id.key, env, cx)
            && let Some(run) = self.runs.get_mut(&id.key)
        {
            note(&mut run.terminal, &format!("couldn't restart: {why}"));
        }
        cx.notify();
    }
}

/// A line devdeck writes into a run's terminal: dim, on a line of its own.
fn note(terminal: &mut Terminal, text: &str) {
    let newline = if terminal.at_line_start() { "" } else { "\r\n" };
    terminal.feed(format!("{newline}\x1b[2m{text}\x1b[0m\r\n").as_bytes());
}

/// A runner with the fake driver, starting the catalog's autostart commands.
pub fn fake_runner(workspace: &str, catalog: Rc<Catalog>, env: Option<String>, cx: &mut gpui::App) -> gpui::Entity<Runner> {
    use gpui::AppContext;
    let auto = catalog.autostart.clone();
    let runner = cx.new(|_| Runner::new(workspace.into(), catalog, Rc::new(crate::fake_driver::FakeDriver)));
    runner.update(cx, |r, cx| {
        for (app, cmd) in auto {
            let _ = r.start(&RunKey::new(&app, &cmd), env.clone(), cx);
        }
    });
    runner
}
