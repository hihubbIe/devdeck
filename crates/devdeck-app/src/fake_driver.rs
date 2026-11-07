//! The demo's run driver: scripted output on timers, written as terminal bytes with
//! colors, through the same path as a real process. Servers keep writing, builds and
//! tests finish, boardservice's server crashes, pagerendererservice's tests fail. A stop
//! takes [`STOP_DELAY`], like a process handling ^C.
//!
//! `DEVDECK_FAKE_RATE=<lines per second>` makes servers write that fast (measurements).

use crate::runner::{Launch, RunControl, RunDriver, RunId, Runner, Started};
use devdeck_core::apps::Tier;
use devdeck_core::runs::{Exit, Level, level_of};
use devdeck_core::terminal::TermSize;
use gpui::{AsyncApp, Context, WeakEntity};
use std::time::Duration;

/// Gap between scripted lines.
pub const LINE_GAP: Duration = Duration::from_millis(200);
/// A server writes one line this often once started.
pub const SERVER_TICK: Duration = Duration::from_secs(2);
/// The demo's crashing server dies this long after starting.
pub const CRASH_AFTER: Duration = Duration::from_secs(3);
/// How long a stop takes.
pub const STOP_DELAY: Duration = Duration::from_millis(300);
/// Output is batched per frame at high rates.
const FRAME: Duration = Duration::from_millis(16);

pub struct FakeDriver;

struct FakeControl {
    stop: async_channel::Sender<()>,
}

impl RunControl for FakeControl {
    fn write(&self, _bytes: &[u8]) {}
    fn resize(&self, _size: TermSize) {}
    fn stop(&self) {
        let _ = self.stop.try_send(());
    }
}

impl RunDriver for FakeDriver {
    fn spawn(&self, launch: Launch, cx: &mut Context<Runner>) -> Result<Started, String> {
        let (lines, exit) = script(&launch);
        let crashes = launch.id.key.app == "boardservice" && exit.is_none();
        let rate = std::env::var("DEVDECK_FAKE_RATE").ok().and_then(|s| s.parse::<u32>().ok()).filter(|r| *r > 0);
        let (tx, rx) = async_channel::bounded(1);
        let id = launch.id;
        let task = cx.spawn(async move |this, cx| {
            let out = Out { id, this, rx };
            for l in lines {
                if out.wait(LINE_GAP, cx).await || !out.line(&l, cx) {
                    return;
                }
            }
            if let Some(code) = exit {
                out.exit(Exit::Code(code), cx);
                return;
            }
            if crashes {
                if out.wait(CRASH_AFTER, cx).await {
                    return;
                }
                out.line("ERR panic: rabbitmq: channel closed", cx);
                out.exit(Exit::Code(2), cx);
                return;
            }
            // Lines owed so far: a fraction carries over, so any rate is exact on average.
            let (tick, per_tick) = match rate {
                Some(r) => (FRAME, r as f64 * FRAME.as_secs_f64()),
                None => (SERVER_TICK, 1.0),
            };
            let (mut n, mut owed) = (0u64, 0.0f64);
            loop {
                if out.wait(tick, cx).await {
                    return;
                }
                owed += per_tick;
                let due = owed.floor();
                owed -= due;
                let mut chunk = String::new();
                for _ in 0..due as u64 {
                    n += 1;
                    chunk.push_str(&colored(&server_line(n)));
                }
                if !chunk.is_empty() && !out.bytes(chunk.as_bytes(), cx) {
                    return;
                }
            }
        });
        Ok(Started { control: Box::new(FakeControl { stop: tx }), task })
    }
}

/// Reports for one run.
struct Out {
    id: RunId,
    this: WeakEntity<Runner>,
    rx: async_channel::Receiver<()>,
}

impl Out {
    /// Waits `d`, or handles a stop: the process "exits" [`STOP_DELAY`] later. True when
    /// it stopped.
    async fn wait(&self, d: Duration, cx: &mut AsyncApp) -> bool {
        let timer = cx.background_executor().timer(d);
        let stopped = futures_lite::future::or(
            async {
                timer.await;
                false
            },
            async { self.rx.recv().await.is_ok() },
        )
        .await;
        if stopped {
            cx.background_executor().timer(STOP_DELAY).await;
            self.line("^C", cx);
            self.exit(Exit::Signal(2), cx);
        }
        stopped
    }

    fn line(&self, text: &str, cx: &mut AsyncApp) -> bool {
        self.bytes(colored(text).as_bytes(), cx)
    }

    fn bytes(&self, bytes: &[u8], cx: &mut AsyncApp) -> bool {
        self.this.update(cx, |r, cx| r.output(&self.id, bytes, cx)).is_ok()
    }

    fn exit(&self, exit: Exit, cx: &mut AsyncApp) {
        let _ = self.this.update(cx, |r, cx| r.exited(&self.id, exit, cx));
    }
}

/// A line as a program would print it: the level word colored, CRLF.
fn colored(line: &str) -> String {
    let color = match level_of(line) {
        Level::Error => "31",
        Level::Warn => "33",
        Level::Info => "32",
        Level::Debug => "90",
        Level::Plain => return format!("{line}\r\n"),
    };
    match line.split_once(' ') {
        Some((head, rest)) if head.len() <= 6 => format!("\x1b[{color}m{head}\x1b[0m {rest}\r\n"),
        _ => format!("\x1b[{color}m{line}\x1b[0m\r\n"),
    }
}

fn server_line(n: u64) -> String {
    match n % 4 {
        0 => "WRN slow query entities.find 812ms".to_string(),
        1 => format!("INF GET /v1/entities/{n} 200 12ms"),
        2 => "DBG cache hit".to_string(),
        _ => "INF POST /v1/entities 201 31ms".to_string(),
    }
}

/// What a command prints and how it ends (`None`: it keeps running).
fn script(launch: &Launch) -> (Vec<String>, Option<i32>) {
    let (cmd, app) = (&launch.cmd, launch.id.key.app.as_str());
    let env = launch.env.as_deref().unwrap_or("-");
    let port = cmd.ports.first().copied().unwrap_or(0);
    let server = cmd.tier == Tier::Primary;
    let lines: Vec<String> = match (cmd.name(), server) {
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
        ("test", _) if app == "pagerendererservice" => vec![
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
    } else if app == "pagerendererservice" && cmd.name() == "test" {
        Some(1)
    } else {
        Some(0)
    };
    (lines, exit)
}
