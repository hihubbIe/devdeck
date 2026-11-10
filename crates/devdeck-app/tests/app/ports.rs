//! M2 step 3: ports of real runs (R6.9): conflicts before a start, "Stop holder", port
//! memory and when the background scan runs. Runs come from a test driver whose process
//! is this test process (or a `sleep` it starts): the ports are real sockets bound here,
//! and stopping a run signals nothing.

use devdeck_app::port_scan::{PortHolder, Ports, SCAN_EVERY, start_or_toast};
use devdeck_app::runner::{Launch, RunControl, RunDriver, RunKey, RunRefusal, Runner, Started};
use devdeck_core::apps::{App, Command, EnvApply, Tier, Tools};
use devdeck_core::catalog::Catalog;
use devdeck_core::ports::{PortMemory, holder};
use devdeck_core::runs::{Exit, RunState};
use devdeck_core::terminal::TermSize;
use devdeck_ui::overlay::{layer, layer_state};
use devdeck_ui::{Theme, render_layer, toasts};
use gpui::{AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, TestAppContext, VisualTestContext, Window, div};
use std::cell::{Cell, RefCell};
use std::net::TcpListener;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

const WS: &str = "ws";

/// The pid runs report (shared: changing it changes a running run's), and a listener the
/// next started run takes and drops when stopped.
#[derive(Clone, Default)]
struct Next {
    pid: Rc<Cell<Option<u32>>>,
    listener: Rc<RefCell<Option<TcpListener>>>,
}

struct TestDriver(Next);

struct TestControl {
    pid: Rc<Cell<Option<u32>>>,
    listener: Rc<RefCell<Option<TcpListener>>>,
    stop: async_channel::Sender<()>,
}

impl RunControl for TestControl {
    fn write(&self, _: &[u8]) {}
    fn resize(&self, _: TermSize) {}
    fn stop(&self) {
        self.listener.borrow_mut().take();
        let _ = self.stop.try_send(());
    }
    fn pid(&self) -> Option<u32> {
        self.pid.get()
    }
}

impl RunDriver for TestDriver {
    fn spawn(&self, launch: Launch, cx: &mut Context<Runner>) -> Result<Started, String> {
        let (tx, rx) = async_channel::unbounded();
        let control = TestControl {
            pid: self.0.pid.clone(),
            listener: Rc::new(RefCell::new(self.0.listener.borrow_mut().take())),
            stop: tx,
        };
        let id = launch.id;
        let task = cx.spawn(async move |this, cx| {
            if rx.recv().await.is_ok() {
                let _ = this.update(cx, |r, cx| r.exited(&id, Exit::Signal(15), cx));
            }
        });
        Ok(Started { control: Box::new(control), task })
    }
}

fn command(id: &str, ports: &[u16]) -> Command {
    Command { id: id.into(), line: format!("serve {id}"), tier: Tier::Primary, tools: Vec::new(), from_install: Vec::new(), setup: false, ports: ports.to_vec() }
}

fn catalog(commands: Vec<Command>) -> Catalog {
    let app = App {
        id: "web".into(),
        name: "web".into(),
        folder: "web".into(),
        dir: std::env::temp_dir(),
        kind: "test".into(),
        library: false,
        facts: Default::default(),
        commands,
        env: EnvApply::None,
        urls: Vec::new(),
        images: Vec::new(),
    };
    Catalog { apps: vec![app], tools: Tools::new(&[]), ..Catalog::default() }
}

struct Setup {
    runner: Entity<Runner>,
    ports: Entity<Ports>,
    next: Next,
    state: PathBuf,
}

fn setup(cx: &mut TestAppContext, tag: &str, commands: Vec<Command>) -> Setup {
    let state = cx.update(|cx| devdeck_app::test_dir(tag, cx));
    let next = Next::default();
    let driver = Rc::new(TestDriver(next.clone()));
    let runner = cx.update(|cx| cx.new(|_| Runner::new(WS.into(), Rc::new(catalog(commands)), driver)));
    let ports = cx.update(|cx| Ports::new(WS, runner.clone(), Some(state.clone()), cx));
    Setup { runner, ports, next, state }
}

fn key(cmd: &str) -> RunKey {
    RunKey::new("web", cmd)
}

/// A process a test started, killed however the test ends.
struct Reap(std::process::Child);

impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bind() -> (TcpListener, u16) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    (l, port)
}

/// Waits until nothing holds `port`. A process another test spawns meanwhile holds a copy
/// of a dropped listener's descriptor from its fork until its exec closes it
/// (close-on-exec): free within moments.
fn wait_free(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while holder(port).is_some() {
        assert!(Instant::now() < deadline, "port {port} free again");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn try_start(s: &Setup, cx: &mut TestAppContext, cmd: &str) -> Result<(), RunRefusal> {
    let r = cx.update(|cx| s.runner.update(cx, |r, cx| r.try_start(&key(cmd), None, cx)));
    cx.run_until_parked();
    r
}

fn state(s: &Setup, cx: &TestAppContext, cmd: &str) -> Option<RunState> {
    cx.read(|cx| s.runner.read(cx).state(&key(cmd)))
}

fn advance(cx: &mut TestAppContext, d: std::time::Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

/// A window drawing the overlay layer, so toast actions and modals have one.
struct Host;

impl Render for Host {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().children(render_layer(window, cx))
    }
}

fn window(cx: &mut TestAppContext) -> AnyWindowHandle {
    cx.update(|cx| cx.set_global(Theme::devdeck_dark(1.0)));
    let w = cx.add_window(|window, cx| {
        let overlays = layer(window.window_handle(), cx);
        cx.observe(&overlays, |_, _, cx| cx.notify()).detach();
        Host
    });
    cx.run_until_parked();
    w.into()
}

/// Runs the last toast's action named `name`.
fn toast_action(cx: &mut TestAppContext, w: AnyWindowHandle, name: &str) {
    let action = cx.update(|cx| {
        let t = toasts(cx).read(cx).queue.items.last().cloned().expect("a toast");
        t.actions.iter().find(|(n, _)| n == name).map(|(_, a)| a.clone()).unwrap_or_else(|| panic!("no {name} on {:?}", t.text))
    });
    cx.update_window(w, |_, window, cx| action(window, cx)).unwrap();
    cx.run_until_parked();
}

fn last_toast(cx: &mut TestAppContext) -> String {
    cx.update(|cx| toasts(cx).read(cx).queue.items.last().map(|t| t.text.to_string()).unwrap_or_default())
}

#[gpui::test]
fn a_start_is_refused_while_its_port_is_held_and_names_the_holder(cx: &mut TestAppContext) {
    let (held, port) = bind();
    let s = setup(cx, "held", vec![command("a:serve", &[port])]);
    let me = std::process::id();
    match try_start(&s, cx, "a:serve") {
        Err(RunRefusal::PortHeld { port: p, holder: PortHolder::Process { pid, .. } }) => {
            assert_eq!((p, pid), (port, me), "the port and the process holding it");
        }
        other => panic!("expected a held port, got {other:?}"),
    }
    let text = cx.update(|cx| s.runner.update(cx, |r, cx| r.start(&key("a:serve"), None, cx))).unwrap_err();
    assert!(text.starts_with(&format!("port {port} is held by ")) && text.ends_with(&format!("(pid {me})")), "{text}");
    assert_eq!(state(&s, cx, "a:serve"), None, "nothing started");

    // A refused start becomes a toast offering to stop the holder: here another process,
    // so a confirmation naming it comes first (never confirmed: it's this test).
    let w = window(cx);
    cx.update(|cx| start_or_toast(&s.runner, &key("a:serve"), None, cx));
    assert_eq!(last_toast(cx), format!("web · serve: {text}"));
    // Not one of devdeck's runs: not offered while edit mode is off (R5.6), and refused
    // if asked anyway (a toast from before).
    let offered = cx.update(|cx| toasts(cx).read(cx).queue.items.last().is_some_and(|t| t.actions.iter().any(|(n, _)| n == "Stop holder")));
    assert!(!offered, "no Stop holder while edit mode is off");
    let holder = PortHolder::Process { pid: me, command: "test".into() };
    let runner = s.runner.downgrade();
    cx.update_window(w, |_, window, cx| devdeck_app::port_scan::stop_holder(&runner, port, &holder, window, cx)).unwrap();
    assert_eq!(cx.update(|cx| layer_state(w, cx)), (false, 0), "no confirmation");
    assert_eq!(last_toast(cx), format!("Stop holder: {}.", devdeck_app::edit_mode::OFF));
    cx.update(|cx| devdeck_app::edit_mode::set(true, cx));
    cx.update(|cx| start_or_toast(&s.runner, &key("a:serve"), None, cx));
    toast_action(cx, w, "Stop holder");
    assert_eq!(cx.update(|cx| layer_state(w, cx)), (false, 1), "a confirmation is open");
    let mut vcx = VisualTestContext::from_window(w, cx);
    assert!(vcx.debug_bounds("stop-holder-command").is_some(), "it shows the command");
    assert!(vcx.debug_bounds("stop-holder-confirm").is_some());

    drop(held);
    wait_free(port);
    assert_eq!(try_start(&s, cx, "a:serve"), Ok(()), "free again");
}

#[gpui::test]
fn a_remembered_port_refuses_a_start_too(cx: &mut TestAppContext) {
    let (_held, port) = bind();
    let s = setup(cx, "remembered", vec![command("a:serve", &[])]);
    PortMemory::load(&s.state).remember(WS, "web", "a:serve", &[port]).unwrap();
    assert!(matches!(try_start(&s, cx, "a:serve"), Err(RunRefusal::PortHeld { port: p, .. }) if p == port));
}

#[gpui::test]
fn stop_holder_on_our_own_run_stops_it(cx: &mut TestAppContext) {
    let (l, port) = bind();
    // The listener is bound before `serve` starts: only `other` declares its port.
    let s = setup(cx, "own", vec![command("a:serve", &[]), command("a:other", &[port])]);
    s.next.pid.set(Some(std::process::id()));
    *s.next.listener.borrow_mut() = Some(l);
    assert_eq!(try_start(&s, cx, "a:serve"), Ok(()));
    match try_start(&s, cx, "a:other") {
        Err(RunRefusal::PortHeld { port: p, holder: PortHolder::Run { key: k, title } }) => {
            assert_eq!((p, k, title.as_str()), (port, key("a:serve"), "web · serve"));
        }
        other => panic!("expected our own run, got {other:?}"),
    }
    let w = window(cx);
    cx.update(|cx| start_or_toast(&s.runner, &key("a:other"), None, cx));
    assert_eq!(last_toast(cx), format!("web · other: port {port} is held by web · serve"));
    toast_action(cx, w, "Stop holder");
    assert_eq!(cx.update(|cx| layer_state(w, cx)), (false, 0), "no confirmation for our own run");
    assert_eq!(state(&s, cx, "a:serve"), Some(RunState::Stopped));
    wait_free(port);
    assert_eq!(try_start(&s, cx, "a:other"), Ok(()));
}

#[gpui::test]
fn an_empty_scan_does_not_erase_remembered_ports(cx: &mut TestAppContext) {
    let (l, port) = bind();
    let s = setup(cx, "memory", vec![command("a:serve", &[])]);
    s.next.pid.set(Some(std::process::id()));
    *s.next.listener.borrow_mut() = Some(l);
    assert_eq!(try_start(&s, cx, "a:serve"), Ok(()));
    // Other tests of this process may listen too: only this port is looked for.
    let found = |cx: &TestAppContext| cx.read(|cx| s.ports.read(cx).scan.listening.get(&key("a:serve")).cloned().unwrap_or_default());
    assert!(found(cx).contains(&port), "scanned at once: {:?}", found(cx));
    assert!(PortMemory::load(&s.state).get(WS, "web", "a:serve").contains(&port), "remembered");

    // The run now is a process listening on nothing (killed however the test ends).
    let sleep = Reap(std::process::Command::new("sleep").arg("30").spawn().unwrap());
    s.next.pid.set(Some(sleep.0.id()));
    advance(cx, SCAN_EVERY);
    let scanned = found(cx);
    drop(sleep);
    assert_eq!(scanned, [] as [u16; 0], "the scan found nothing");
    assert!(PortMemory::load(&s.state).get(WS, "web", "a:serve").contains(&port), "still remembered");
}

#[gpui::test]
fn every_port_a_run_listened_on_is_remembered(cx: &mut TestAppContext) {
    use std::io::BufRead;
    let (l, port) = bind();
    let s = setup(cx, "union", vec![command("a:serve", &[])]);
    s.next.pid.set(Some(std::process::id()));
    *s.next.listener.borrow_mut() = Some(l);
    assert_eq!(try_start(&s, cx, "a:serve"), Ok(()));
    // The run now is another process, listening on another port only: a scan sees a part.
    let script = "import socket,time\ns=socket.socket()\ns.bind(('127.0.0.1',0))\ns.listen()\nprint(s.getsockname()[1],flush=True)\ntime.sleep(30)";
    let mut server = Reap(std::process::Command::new("python3").args(["-c", script]).stdout(std::process::Stdio::piped()).spawn().unwrap());
    let mut line = String::new();
    std::io::BufReader::new(server.0.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let other: u16 = line.trim().parse().unwrap();
    s.next.pid.set(Some(server.0.id()));
    advance(cx, SCAN_EVERY);
    let found = cx.read(|cx| s.ports.read(cx).scan.listening.get(&key("a:serve")).cloned().unwrap_or_default());
    assert_eq!(found, [other], "this scan sees the other port only");
    let remembered = PortMemory::load(&s.state).get(WS, "web", "a:serve").to_vec();
    assert!(remembered.contains(&port) && remembered.contains(&other), "both kept: {remembered:?}");
}

#[gpui::test]
fn the_scan_runs_only_while_a_run_runs_or_a_port_view_is_shown(cx: &mut TestAppContext) {
    let s = setup(cx, "idle", vec![command("a:serve", &[])]);
    let scans = |cx: &TestAppContext| cx.read(|cx| (s.ports.read(cx).scanning(), s.ports.read(cx).scans()));
    advance(cx, SCAN_EVERY * 3);
    assert_eq!(scans(cx), (false, 0), "nothing runs, no port view");

    s.next.pid.set(Some(std::process::id()));
    assert_eq!(try_start(&s, cx, "a:serve"), Ok(()));
    assert_eq!(scans(cx), (true, 1), "a run started: scanned at once");
    advance(cx, SCAN_EVERY);
    assert_eq!(scans(cx), (true, 2), "then every {SCAN_EVERY:?}");
    cx.update(|cx| s.runner.update(cx, |r, cx| r.stop(&key("a:serve"), cx)));
    cx.run_until_parked();
    assert_eq!(state(&s, cx, "a:serve"), Some(RunState::Stopped));
    advance(cx, SCAN_EVERY);
    assert_eq!(scans(cx), (false, 2), "stops once nothing runs");
    advance(cx, SCAN_EVERY * 3);
    assert_eq!(scans(cx), (false, 2));

    // A port view on screen: who holds its port is looked up.
    let (_held, port) = bind();
    let view = s.ports.entity_id();
    cx.update(|cx| s.ports.update(cx, |p, cx| p.show(view, port, cx)));
    cx.run_until_parked();
    assert_eq!(scans(cx), (true, 3));
    let held = cx.read(|cx| s.ports.read(cx).scan.held.get(&port).cloned());
    assert!(matches!(held, Some(PortHolder::Process { pid, .. }) if pid == std::process::id()), "{held:?}");
    cx.update(|cx| s.ports.update(cx, |p, _| p.hide(view)));
    advance(cx, SCAN_EVERY * 3);
    assert_eq!(scans(cx), (false, 3), "stops once the view is gone");
}

#[gpui::test]
fn a_terminal_listening_is_scanned_without_being_remembered(cx: &mut TestAppContext) {
    let (l, port) = bind();
    let s = setup(cx, "terminal-ports", vec![command("a:serve", &[])]);
    s.next.pid.set(Some(std::process::id()));
    *s.next.listener.borrow_mut() = Some(l);
    let key = cx.update(|cx| s.runner.update(cx, |r, _| r.open_terminal(1, &std::env::temp_dir())));
    let started = cx.update(|cx| s.runner.update(cx, |r, cx| r.try_start(&key, None, cx)));
    assert_eq!(started, Ok(()), "no port check for a terminal");
    advance(cx, SCAN_EVERY);
    advance(cx, SCAN_EVERY);
    let found = cx.read(|cx| s.ports.read(cx).scan.listening.get(&key).cloned().unwrap_or_default());
    assert!(found.contains(&port), "scanned (it shows who holds the port): {found:?}");
    let memory = PortMemory::load(&s.state);
    assert!(memory.get(WS, "", "term/1").is_empty(), "not remembered: what a terminal runs changes");
}

/// The control API's interest (`port.list`, `port.get`, a `ports` follower): scans while
/// held with something to scan, its ports watched; stops once dropped.
#[gpui::test]
fn an_interest_scans_its_ports_while_held(cx: &mut TestAppContext) {
    let s = setup(cx, "interest", vec![command("a:serve", &[])]);
    let scans = |cx: &TestAppContext| cx.read(|cx| (s.ports.read(cx).scanning(), s.ports.read(cx).scans()));
    let idle = cx.update(|cx| s.ports.update(cx, |p, cx| p.interest(Default::default(), cx)));
    advance(cx, SCAN_EVERY);
    assert_eq!(scans(cx), (false, 0), "nothing to scan: no scan");
    drop(idle);
    let (_held, port) = bind();
    let interest = cx.update(|cx| s.ports.update(cx, |p, cx| p.interest(std::collections::BTreeSet::from([port]), cx)));
    cx.run_until_parked();
    assert_eq!(scans(cx), (true, 1));
    let held = cx.read(|cx| s.ports.read(cx).scan.held.get(&port).cloned());
    assert!(matches!(held, Some(PortHolder::Process { pid, .. }) if pid == std::process::id()), "{held:?}");
    drop(interest);
    advance(cx, SCAN_EVERY * 3);
    assert_eq!(scans(cx), (false, 1), "stops once the interest is gone");
}
