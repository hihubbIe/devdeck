//! The runs of one workspace (R6.7): start, stop, restart, state, output. The work
//! itself is done by a [`RunDriver`]: M1 uses [`FakeDriver`] (scripted output on
//! timers); M2 plugs in real processes behind the same trait, and views don't change.

use devdeck_core::apps::{Command, Tools};
use devdeck_core::demo::Catalog;
use devdeck_core::runs::{Output, RunState};
use gpui::{AppContext, Context, EventEmitter, Task};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

/// Gap between scripted lines.
pub const LINE_GAP: Duration = Duration::from_millis(200);
/// A server writes one line this often once started.
pub const SERVER_TICK: Duration = Duration::from_secs(2);
/// The demo's crashing server dies this long after starting.
pub const CRASH_AFTER: Duration = Duration::from_secs(3);
/// How long a stop takes.
pub const STOP_DELAY: Duration = Duration::from_millis(300);
/// Lines kept per run (scrollback).
pub const OUTPUT_CAP: usize = 5000;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunKey {
    pub app: String,
    pub cmd: String,
}

impl RunKey {
    pub fn new(app: &str, cmd: &str) -> Self {
        Self { app: app.into(), cmd: cmd.into() }
    }

    pub fn title(&self) -> String {
        format!("{} · {}", self.app, self.cmd)
    }
}

pub struct Run {
    pub state: RunState,
    pub output: Output,
    pub env: Option<String>,
    pub runs: u32,
    task: Option<Task<()>>,
    restart_after_stop: bool,
}

pub enum RunnerEvent {
    /// A run ended on its own with a non-zero code.
    Failed { key: RunKey, code: i32 },
    /// New output for a run. Lines are events, not notifications, so only views
    /// showing that run redraw; state changes notify.
    Output { key: RunKey },
}

/// Starts the work of a run. The driver reports back through [`Runner::line`] and
/// [`Runner::exited`]; dropping the returned task stops it.
pub trait RunDriver {
    fn spawn(&self, key: RunKey, cmd: &Command, env: Option<String>, cx: &mut Context<Runner>) -> Task<()>;
}

pub struct Runner {
    pub workspace: String,
    catalog: Rc<Catalog>,
    runs: BTreeMap<RunKey, Run>,
    driver: Rc<dyn RunDriver>,
}

impl EventEmitter<RunnerEvent> for Runner {}

impl Runner {
    pub fn new(workspace: String, catalog: Rc<Catalog>, driver: Rc<dyn RunDriver>) -> Self {
        Self { workspace, catalog, runs: BTreeMap::new(), driver }
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
        self.runs.iter().filter(|(_, r)| r.state.is_running()).map(|(k, _)| k.title()).collect()
    }

    /// Running keys with the ports their command listens on.
    pub fn running_ports(&self) -> Vec<(RunKey, u16)> {
        self.runs
            .iter()
            .filter(|(_, r)| r.state == RunState::Running)
            .flat_map(|(k, _)| {
                let ports = self.catalog.app(&k.app).and_then(|a| a.command(&k.cmd)).map(|c| c.ports.clone()).unwrap_or_default();
                ports.into_iter().map(move |p| (k.clone(), p))
            })
            .collect()
    }

    /// Why `key` can't start now, if it can't.
    pub fn refusal(&self, key: &RunKey) -> Option<String> {
        let Some(cmd) = self.catalog.app(&key.app).and_then(|a| a.command(&key.cmd)) else {
            return Some(format!("{} has no command {}", key.app, key.cmd));
        };
        if let Some(missing) = self.catalog.tools.missing(cmd) {
            return Some(missing);
        }
        (self.state(key) == Some(RunState::Running)).then(|| "already running".to_string())
    }

    /// Starts a command with the workspace env. Starting while it stops restarts it once
    /// the stop is done.
    pub fn start(&mut self, key: &RunKey, env: Option<String>, cx: &mut Context<Self>) -> Result<(), String> {
        if self.state(key) == Some(RunState::Stopping) {
            if let Some(r) = self.runs.get_mut(key) {
                r.restart_after_stop = true;
            }
            return Ok(());
        }
        if let Some(why) = self.refusal(key) {
            return Err(why);
        }
        let cmd = self.catalog.app(&key.app).and_then(|a| a.command(&key.cmd)).cloned().expect("checked");
        let run = self.runs.entry(key.clone()).or_insert_with(|| Run {
            state: RunState::Stopped,
            output: Output::new(OUTPUT_CAP),
            env: None,
            runs: 0,
            task: None,
            restart_after_stop: false,
        });
        run.runs += 1;
        run.state = RunState::Running;
        run.env = env.clone();
        run.restart_after_stop = false;
        let header = format!("── run {} · {}{} ──", run.runs, cmd.line, env.as_deref().map(|e| format!(" · @{e}")).unwrap_or_default());
        run.output.push(header);
        let task = self.driver.spawn(key.clone(), &cmd, env, cx);
        if let Some(r) = self.runs.get_mut(key) {
            r.task = Some(task);
        }
        cx.notify();
        Ok(())
    }

    /// Asks a run to stop; it reports Stopped after [`STOP_DELAY`].
    pub fn stop(&mut self, key: &RunKey, cx: &mut Context<Self>) {
        let Some(run) = self.runs.get_mut(key) else { return };
        if run.state != RunState::Running {
            return;
        }
        run.state = RunState::Stopping;
        run.task = None;
        let key = key.clone();
        run.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STOP_DELAY).await;
            let _ = this.update(cx, |r, cx| {
                let mut again = None;
                if let Some(run) = r.runs.get_mut(&key) {
                    run.state = RunState::Stopped;
                    run.task = None;
                    run.output.push("^C stopped");
                    if run.restart_after_stop {
                        again = Some(run.env.clone());
                    }
                }
                if let Some(env) = again {
                    let _ = r.start(&key, env, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn restart(&mut self, key: &RunKey, env: Option<String>, cx: &mut Context<Self>) -> Result<(), String> {
        match self.state(key) {
            Some(RunState::Running) => {
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

    pub fn stop_all(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<RunKey> = self.runs.iter().filter(|(_, r)| r.state == RunState::Running).map(|(k, _)| k.clone()).collect();
        for k in keys {
            self.stop(&k, cx);
            if let Some(r) = self.runs.get_mut(&k) {
                r.restart_after_stop = false;
            }
        }
    }

    /// Driver: one line of output.
    pub fn line(&mut self, key: &RunKey, text: impl Into<String>, cx: &mut Context<Self>) {
        if let Some(r) = self.runs.get_mut(key)
            && r.state == RunState::Running
        {
            r.output.push(text);
            cx.emit(RunnerEvent::Output { key: key.clone() });
        }
    }

    /// Driver: the run ended on its own.
    pub fn exited(&mut self, key: &RunKey, code: i32, cx: &mut Context<Self>) {
        let Some(r) = self.runs.get_mut(key) else { return };
        if r.state != RunState::Running {
            return;
        }
        r.state = RunState::Exited(code);
        r.task = None;
        r.output.push(format!("exited with code {code}"));
        if code != 0 {
            cx.emit(RunnerEvent::Failed { key: key.clone(), code });
        }
        cx.notify();
    }
}

/// Scripted output for the demo: servers keep writing, builds and tests finish, the
/// boardservice server crashes, pagerendererservice's tests fail.
pub struct FakeDriver;

impl FakeDriver {
    fn script(key: &RunKey, cmd: &Command, env: Option<&str>) -> (Vec<String>, Option<i32>) {
        let env = env.unwrap_or("-");
        let port = cmd.ports.first().copied().unwrap_or(0);
        let server = cmd.tier == devdeck_core::apps::Tier::Primary;
        let lines: Vec<String> = match (cmd.id.as_str(), server) {
            ("run", true) if cmd.line.starts_with("go") => vec![
                "go: downloading modules".into(),
                format!("INF config loaded env={env}"),
                "INF connected to mongodb".into(),
                format!("INF listening on :{port}"),
            ],
            (_, true) if cmd.line.starts_with("pnpm") => vec![
                format!("> {}", cmd.line),
                "  VITE v5.4.2  ready in 812 ms".into(),
                format!("  ➜  Local:   http://localhost:{port}/"),
            ],
            (_, true) => vec![format!("$ {}", cmd.line), "Compiling…".into(), "Finished in 4.1s".into()],
            ("test", _) if key.app == "pagerendererservice" => vec![
                "=== RUN   TestRender".into(),
                "--- FAIL: TestRender (0.21s)".into(),
                "    render_test.go:42: error: expected 3 blocks, got 2".into(),
                "FAIL".into(),
            ],
            ("test", _) => vec!["=== RUN   TestAll".into(), "ok   ./internal/... 0.41s".into(), "ok   ./pkg/... 0.18s".into(), "PASS".into()],
            ("build", _) => vec![format!("$ {}", cmd.line), "WARN 2 deprecated packages".into(), "built in 3.2s".into()],
            _ => vec![format!("$ {}", cmd.line), "done".into()],
        };
        let exit = if server {
            None
        } else if key.app == "pagerendererservice" && cmd.id == "test" {
            Some(1)
        } else {
            Some(0)
        };
        (lines, exit)
    }
}

impl RunDriver for FakeDriver {
    fn spawn(&self, key: RunKey, cmd: &Command, env: Option<String>, cx: &mut Context<Runner>) -> Task<()> {
        let (lines, exit) = Self::script(&key, cmd, env.as_deref());
        let crashes = key.app == "boardservice" && exit.is_none();
        cx.spawn(async move |this, cx| {
            for l in lines {
                cx.background_executor().timer(LINE_GAP).await;
                if this.update(cx, |r, cx| r.line(&key, l, cx)).is_err() {
                    return;
                }
            }
            if let Some(code) = exit {
                let _ = this.update(cx, |r, cx| r.exited(&key, code, cx));
                return;
            }
            if crashes {
                cx.background_executor().timer(CRASH_AFTER).await;
                let _ = this.update(cx, |r, cx| {
                    r.line(&key, "ERR panic: rabbitmq: channel closed", cx);
                    r.exited(&key, 2, cx);
                });
                return;
            }
            let mut n = 0u64;
            loop {
                cx.background_executor().timer(SERVER_TICK).await;
                n += 1;
                let text = match n % 4 {
                    0 => "WRN slow query entities.find 812ms".to_string(),
                    1 => format!("INF GET /v1/entities/{n} 200 12ms"),
                    2 => "DBG cache hit".to_string(),
                    _ => "INF POST /v1/entities 201 31ms".to_string(),
                };
                if this.update(cx, |r, cx| r.line(&key, text, cx)).is_err() {
                    return;
                }
            }
        })
    }
}

/// A runner with the fake driver, starting the catalog's autostart commands.
pub fn fake_runner(workspace: &str, catalog: Rc<Catalog>, env: Option<String>, cx: &mut gpui::App) -> gpui::Entity<Runner> {
    let auto = catalog.autostart.clone();
    let runner = cx.new(|_| Runner::new(workspace.into(), catalog, Rc::new(FakeDriver)));
    runner.update(cx, |r, cx| {
        for (app, cmd) in auto {
            let _ = r.start(&RunKey::new(&app, &cmd), env.clone(), cx);
        }
    });
    runner
}
