//! M2 step 3: real runs. A runner with the pseudo-terminal driver runs real `sh` children
//! on GPUI's test platform; every test's processes end within it.
//!
//! The runner's side runs on the test's fake clock, the processes in real time: tests
//! allow parking (the run's thread wakes the driver's task) and poll, advancing the clock
//! by the driver's [`FRAME`] and sleeping a little, until what they wait for happens.

use devdeck_app::environment::{EnvState, Environment};
use devdeck_app::pty_driver::{FRAME, PtyDriver};
use devdeck_app::runner::{RunKey, Runner};
use devdeck_core::apps::{App, Command, EnvApply, Tier, Tools};
use devdeck_core::catalog::Catalog;
use devdeck_core::environment::Toolbox;
use devdeck_core::runs::RunState;
use devdeck_core::terminal::TermSize;
use gpui::{AppContext, Entity, TestAppContext};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

const APP: &str = "probe";

struct Setup {
    runner: Entity<Runner>,
    /// The PATH the toolbox gives.
    path: String,
}

/// A runner over one app in `dir`, with a command per `(name, line)` (`sh:<name>`), the
/// env bound as `APP_ENV`, and a toolbox of a few variables of its own.
fn setup(cx: &mut TestAppContext, dir: Option<PathBuf>, commands: &[(&str, &str)]) -> Setup {
    cx.executor().allow_parking();
    let home = cx.update(|cx| devdeck_app::test_dir("pty", cx));
    let path = format!("{}/bin:/usr/bin:/bin", home.display());
    let vars = vec![("HOME".to_string(), home.display().to_string()), ("PATH".into(), path.clone()), ("FROM_TOOLBOX".into(), "yes".into())];
    let toolbox = Toolbox { vars, ..Toolbox::default() };
    let commands = commands
        .iter()
        .map(|(name, line)| Command { id: format!("sh:{name}"), line: line.to_string(), tier: Tier::Secondary, tools: Vec::new(), from_install: Vec::new(), setup: false, ports: Vec::new() })
        .collect();
    let app = App {
        id: APP.into(),
        name: APP.into(),
        folder: "~/probe".into(),
        dir: dir.unwrap_or(home),
        kind: "sh".into(),
        library: false,
        facts: Default::default(),
        commands,
        env: EnvApply::Variable("APP_ENV".into()),
        urls: Vec::new(),
    };
    let catalog = Catalog { apps: vec![app], tools: Tools { toolbox: Some(toolbox.clone()), ..Tools::default() }, ..Catalog::default() };
    let runner = cx.update(|cx| {
        let env = cx.new(|_| Environment::new(EnvState::Ready(toolbox)));
        cx.new(|_| Runner::new("test".into(), Rc::new(catalog), Rc::new(PtyDriver { env, log: None })))
    });
    Setup { runner, path }
}

fn key(name: &str) -> RunKey {
    RunKey::new(APP, &format!("sh:{name}"))
}

fn start(s: &Setup, cx: &mut TestAppContext, name: &str) {
    cx.update(|cx| s.runner.update(cx, |r, cx| r.start(&key(name), Some("dev".into()), cx))).unwrap();
}

fn state(s: &Setup, cx: &TestAppContext, name: &str) -> Option<RunState> {
    cx.read(|cx| s.runner.read(cx).state(&key(name)))
}

/// The run's terminal rows, without trailing blanks.
fn lines(s: &Setup, cx: &TestAppContext, name: &str) -> Vec<String> {
    cx.read(|cx| s.runner.read(cx).run(&key(name)).map(|r| r.terminal.lines()).unwrap_or_default())
        .into_iter()
        .map(|l| l.trim_end().to_string())
        .collect()
}

fn shows(s: &Setup, cx: &TestAppContext, name: &str, line: &str) -> bool {
    lines(s, cx, name).iter().any(|l| l == line)
}

/// Lets real time pass until `done`, for a few seconds at most.
fn wait_until(cx: &mut TestAppContext, what: &str, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.executor().advance_clock(FRAME);
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_ended(s: &Setup, cx: &mut TestAppContext, name: &str) -> RunState {
    wait_until(cx, "the exit", |cx| !state(s, cx, name).unwrap().is_running());
    state(s, cx, name).unwrap()
}

#[gpui::test]
fn output_reaches_the_terminal(cx: &mut TestAppContext) {
    let s = setup(cx, None, &[("hello", "printf 'hello from %s\\n' devdeck")]);
    start(&s, cx, "hello");
    assert_eq!(wait_ended(&s, cx, "hello"), RunState::Exited(0));
    assert!(shows(&s, cx, "hello", "hello from devdeck"), "{:?}", lines(&s, cx, "hello"));
}

#[gpui::test]
fn the_exit_code_reaches_the_state(cx: &mut TestAppContext) {
    let s = setup(cx, None, &[("fail", "exit 3")]);
    start(&s, cx, "fail");
    assert_eq!(wait_ended(&s, cx, "fail"), RunState::Exited(3));
}

#[gpui::test]
fn a_stop_ends_in_stopped_with_nothing_left(cx: &mut TestAppContext) {
    let s = setup(cx, None, &[("wait", "echo started; sleep 5")]);
    start(&s, cx, "wait");
    wait_until(cx, "the first line", |cx| shows(&s, cx, "wait", "started"));
    let pids = cx.read(|cx| s.runner.read(cx).pids());
    assert_eq!(pids.len(), 1, "a real run has a pid");
    cx.update(|cx| s.runner.update(cx, |r, cx| r.stop(&key("wait"), cx)));
    assert_eq!(state(&s, cx, "wait"), Some(RunState::Stopping));
    assert_eq!(wait_ended(&s, cx, "wait"), RunState::Stopped);
    assert!(!PathBuf::from(format!("/proc/{}", pids[0].1)).exists(), "the process is gone");
    assert!(cx.read(|cx| s.runner.read(cx).pids()).is_empty());
}

#[gpui::test]
fn the_environment_is_exactly_the_runs(cx: &mut TestAppContext) {
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some(), "the test process has variables of its own");
    // One name per line, sorted; sh sets PWD itself.
    let names = "env | sed -e 's/=.*//' -e '/^PWD$/d' | sort | tr '\\n' ' '; echo";
    let s = setup(cx, None, &[("names", names), ("vars", "printf '%s|%s|%s|%s\\n' \"$PATH\" \"$TERM\" \"$COLORTERM\" \"$APP_ENV\"")]);
    start(&s, cx, "names");
    start(&s, cx, "vars");
    assert_eq!(wait_ended(&s, cx, "names"), RunState::Exited(0));
    assert_eq!(wait_ended(&s, cx, "vars"), RunState::Exited(0));
    assert!(shows(&s, cx, "names", "APP_ENV COLORTERM FROM_TOOLBOX HOME PATH TERM"), "{:?}", lines(&s, cx, "names"));
    let expected = format!("{}|xterm-256color|truecolor|dev", s.path);
    assert!(shows(&s, cx, "vars", &expected), "{:?}", lines(&s, cx, "vars"));
}

#[gpui::test]
fn input_reaches_the_process(cx: &mut TestAppContext) {
    let s = setup(cx, None, &[("ask", "read answer; echo \"got $answer\"")]);
    start(&s, cx, "ask");
    cx.update(|cx| s.runner.update(cx, |r, _| r.write(&key("ask"), b"yes\r")));
    assert_eq!(wait_ended(&s, cx, "ask"), RunState::Exited(0));
    assert!(shows(&s, cx, "ask", "got yes"), "{:?}", lines(&s, cx, "ask"));
}

#[gpui::test]
fn a_resize_reaches_the_process(cx: &mut TestAppContext) {
    let s = setup(cx, None, &[("size", "stty size; read go; stty size")]);
    start(&s, cx, "size");
    // The launch size first (the runner's default), then the new one.
    wait_until(cx, "the first size", |cx| shows(&s, cx, "size", "30 120"));
    cx.update(|cx| {
        s.runner.update(cx, |r, cx| {
            r.resize(&key("size"), TermSize { cols: 100, rows: 40 }, cx);
            r.write(&key("size"), b"\r");
        })
    });
    assert_eq!(wait_ended(&s, cx, "size"), RunState::Exited(0));
    assert!(shows(&s, cx, "size", "40 100"), "{:?}", lines(&s, cx, "size"));
}

#[gpui::test]
fn a_missing_folder_is_not_started_with_the_reason(cx: &mut TestAppContext) {
    let missing = cx.update(|cx| devdeck_app::test_dir("pty-missing", cx)).join("gone");
    let s = setup(cx, Some(missing.clone()), &[("hello", "echo hello")]);
    start(&s, cx, "hello");
    assert_eq!(state(&s, cx, "hello"), Some(RunState::NotStarted));
    let error = cx.read(|cx| s.runner.read(cx).run(&key("hello")).unwrap().error.clone());
    assert_eq!(error, Some(format!("{}: no such folder", missing.display())));
}
