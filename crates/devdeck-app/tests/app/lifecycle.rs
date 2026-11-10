//! M2 step 3: lifecycle of real processes (R8.1, R8.2): one devdeck per state folder,
//! processes a previous devdeck left offered for killing, the quit wait following the
//! slowest stop sequence. Tests only signal processes they start.

use devdeck_app::lifecycle::{KillLeftovers, Leftovers, claim, offer_leftovers};
use devdeck_app::runner::{Launch, RunControl, RunDriver, RunKey, Runner, Started};
use devdeck_app::shell::Shell;
use devdeck_core::apps::{App as AppDef, Command, EnvApply, Tier, Tools};
use devdeck_core::catalog::Catalog;
use devdeck_core::process::{GroupLog, Process, Spawn, StopKind};
use devdeck_core::scope::Scope;
use devdeck_core::terminal::TermSize;
use gpui::{AppContext, TestAppContext};
use std::rc::Rc;
use std::time::{Duration, Instant};

#[gpui::test]
fn one_devdeck_per_state_folder(cx: &mut TestAppContext) {
    let dir = cx.update(|cx| devdeck_app::test_dir("lock", cx));
    let first = claim(&dir).expect("free");
    let second = claim(&dir).err().expect("held");
    assert_eq!(second, format!("Kugelblitz is already running (pid {})", std::process::id()));
    drop(first);
    // A process another test spawns meanwhile holds a copy of the lock's descriptor
    // from its fork until its exec closes it (close-on-exec): free within moments.
    let deadline = Instant::now() + Duration::from_secs(2);
    while claim(&dir).is_err() {
        assert!(Instant::now() < deadline, "free again");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// A `sleep` that isn't a child of this process (its shell exits), recorded in the log
/// as a previous devdeck would have. Killed when dropped, whatever the test did.
struct Orphan(u32);

impl Orphan {
    fn start(log: &GroupLog) -> Orphan {
        let out = std::process::Command::new("sh").args(["-c", "sleep 30 >/dev/null 2>&1 & echo $!"]).output().unwrap();
        let pid: u32 = String::from_utf8(out.stdout).unwrap().trim().parse().unwrap();
        let start = devdeck_sys::procs::get(pid).unwrap().start;
        std::fs::create_dir_all(log.path.parent().unwrap()).unwrap();
        std::fs::write(&log.path, format!("{pid} {start}\n")).unwrap();
        Orphan(pid)
    }

    fn alive(&self) -> bool {
        devdeck_sys::procs::get(self.0).is_some_and(|p| !p.ended)
    }
}

impl Drop for Orphan {
    fn drop(&mut self) {
        devdeck_core::process::kill_all(&[self.0]);
    }
}

#[gpui::test]
fn processes_a_previous_devdeck_left_are_offered_and_killed(cx: &mut TestAppContext) {
    let config = cx.update(|cx| devdeck_app::test_dir("config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let state = cx.update(|cx| devdeck_app::test_dir("state", cx));
    let log = GroupLog::new(&state);
    let orphan = Orphan::start(&log);
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.update(|cx| offer_leftovers(&shell, &log, cx));
    cx.run_until_parked();
    let main = cx.read(|cx| shell.read(cx).main_window().unwrap());
    assert_eq!(cx.update(|cx| devdeck_ui::overlay::layer_state(main.into(), cx)), (false, 1), "the dialog");
    cx.update_window(main.into(), |_, window, cx| window.dispatch_action(Box::new(KillLeftovers), cx)).unwrap();
    cx.run_until_parked();
    let deadline = Instant::now() + Duration::from_secs(2);
    while orphan.alive() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!orphan.alive(), "killed");
    assert_eq!(cx.update(|cx| devdeck_ui::overlay::layer_state(main.into(), cx)), (false, 0), "closed");
}

#[gpui::test]
fn leftovers_left_alone_stay_alone(cx: &mut TestAppContext) {
    let config = cx.update(|cx| devdeck_app::test_dir("config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let state = cx.update(|cx| devdeck_app::test_dir("state", cx));
    let log = GroupLog::new(&state);
    let orphan = Orphan::start(&log);
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.update(|cx| offer_leftovers(&shell, &log, cx));
    cx.run_until_parked();
    let main = cx.read(|cx| shell.read(cx).main_window().unwrap());
    cx.update_window(main.into(), |_, window, cx| window.dispatch_action(Box::new(devdeck_ui::overlay::ModalClose), cx)).unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| devdeck_ui::overlay::layer_state(main.into(), cx)), (false, 0), "closed");
    assert!(orphan.alive());
    assert_eq!(log.live(), [] as [u32; 0], "forgotten: quitting this devdeck won't kill it");
}

#[gpui::test]
fn a_scope_a_killed_devdeck_left_is_one_row_named_after_its_run(cx: &mut TestAppContext) {
    if devdeck_core::scope::systemd_run().is_none() {
        eprintln!("no systemd user manager here: scope test skipped");
        return;
    }
    let config = cx.update(|cx| devdeck_app::test_dir("config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let state = cx.update(|cx| devdeck_app::test_dir("state", cx));
    let log = GroupLog::new(&state);
    // A run in a scope, with a daemon out of its tree, recorded as by a devdeck that's gone.
    let spawn = Spawn {
        line: "( setsid sleep 300 & ); echo ready; sleep 300".into(),
        dir: std::env::temp_dir(),
        vars: vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())],
        size: TermSize { cols: 80, rows: 24 },
        stop: StopKind::Default,
        scope: true,
        label: "web · dev".into(),
    };
    let (process, _rx) = Process::spawn(spawn, Some(log.clone())).expect("spawns");
    let deadline = Instant::now() + Duration::from_secs(3);
    let recorded = || std::fs::read_to_string(&log.path).unwrap_or_default();
    while !recorded().lines().any(|l| l.starts_with("scope ")) || Scope::named(scope_unit(&recorded())).pids().len() < 3 {
        assert!(Instant::now() < deadline, "the run's scope: {}", recorded());
        std::thread::sleep(Duration::from_millis(10));
    }
    let gone: String = recorded()
        .lines()
        .map(|l| match l.strip_prefix("scope ") {
            Some(rest) => {
                let mut parts = rest.splitn(3, ' ');
                let (unit, _, label) = (parts.next().unwrap(), parts.next(), parts.next().unwrap_or_default());
                format!("scope {unit} 999999999-1 {label}\n")
            }
            None => format!("{l}\n"),
        })
        .collect();
    std::fs::write(&log.path, gone).unwrap();
    let scope = Scope::named(scope_unit(&recorded()));

    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.update(|cx| offer_leftovers(&shell, &log, cx));
    cx.run_until_parked();
    let main = cx.read(|cx| shell.read(cx).main_window().unwrap());
    let dialog = cx.update(|cx| devdeck_ui::overlay::layer(main.into(), cx).read(cx).modals.last().cloned()).expect("the dialog");
    let rows = cx.update(|cx| dialog.downcast::<Leftovers>().unwrap().read(cx).rows.clone());
    assert_eq!(rows.len(), 1, "one row for the run: {rows:?}");
    assert!(rows[0].starts_with("web · dev: ") && rows[0].contains("sleep 300"), "{rows:?}");
    cx.update_window(main.into(), |_, window, cx| window.dispatch_action(Box::new(KillLeftovers), cx)).unwrap();
    cx.run_until_parked();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !scope.is_empty() {
        assert!(Instant::now() < deadline, "the scope emptied");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(process);
}

/// The unit of the (only) scope line of a group log.
fn scope_unit(log: &str) -> String {
    log.lines().find_map(|l| l.strip_prefix("scope ")).and_then(|r| r.split(' ').next()).unwrap_or_default().to_string()
}

/// Runs that claim a process id (never signaled: these tests don't quit).
struct Claims;

struct Pid;

impl RunControl for Pid {
    fn write(&self, _: &[u8]) {}
    fn resize(&self, _: TermSize) {}
    fn stop(&self) {}
    fn pid(&self) -> Option<u32> {
        Some(u32::MAX)
    }
}

impl RunDriver for Claims {
    fn spawn(&self, _: Launch, _: &mut gpui::Context<Runner>) -> Result<Started, String> {
        Ok(Started { control: Box::new(Pid), task: gpui::Task::ready(()) })
    }
}

fn command(id: &str, tools: &[&str]) -> Command {
    Command { id: id.into(), line: id.into(), tier: Tier::Primary, tools: tools.iter().map(|t| t.to_string()).collect(), from_install: vec![], setup: false, ports: vec![] }
}

#[gpui::test]
fn quitting_waits_for_the_slowest_stop_sequence(cx: &mut TestAppContext) {
    let app = AppDef {
        id: "stack".into(),
        name: "stack".into(),
        folder: "/stack".into(),
        dir: "/stack".into(),
        kind: "docker".into(),
        library: false,
        facts: Default::default(),
        commands: vec![command("compose:up", &["docker"]), command("make:dev", &["make"])],
        env: EnvApply::None,
        urls: vec![],
        images: vec![],
    };
    let catalog = Rc::new(Catalog { apps: vec![app], tools: Tools::new(&["docker", "make"]), ..Catalog::default() });
    let runner = cx.new(|_| Runner::new("work".into(), catalog, Rc::new(Claims)));
    let start = |cmd: &str, cx: &mut TestAppContext| cx.update(|cx| runner.update(cx, |r, cx| r.start(&RunKey::new("stack", cmd), None, cx))).unwrap();
    assert_eq!(cx.read(|cx| runner.read(cx).longest_stop()), Duration::ZERO, "nothing runs");
    start("make:dev", cx);
    assert_eq!(cx.read(|cx| runner.read(cx).longest_stop()), Duration::from_secs(5), "a dev server: ^C, 3 s, SIGTERM, 2 s");
    start("compose:up", cx);
    assert_eq!(cx.read(|cx| runner.read(cx).longest_stop()), Duration::from_secs(20), "compose stops its containers");
}

/// A shell that quits for real, its workspace's demo runs going.
fn quitting_shell(cx: &mut TestAppContext, tag: &str) -> gpui::Entity<Shell> {
    let dir = cx.update(|cx| devdeck_app::test_dir(tag, cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let shell = cx.update(|cx| Shell::start_single(devdeck_core::layout::Layout::new, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = true));
    shell
}

#[gpui::test]
fn a_quit_from_outside_the_shell_kills_everything(cx: &mut TestAppContext) {
    // The Dock's Quit, logout, shutdown: GPUI quits without asking the shell.
    let shell = quitting_shell(cx, "quit-outside");
    cx.quit();
    assert_eq!(cx.read(|cx| shell.read(cx).kills), 1);
}

#[gpui::test]
fn a_quit_after_a_signal_kills_once(cx: &mut TestAppContext) {
    let shell = quitting_shell(cx, "quit-signal");
    cx.update(|cx| shell.update(cx, |s, cx| s.kill_to_quit(cx)));
    cx.quit();
    assert_eq!(cx.read(|cx| shell.read(cx).kills), 1, "the signal's kill only");
}

#[gpui::test]
fn tests_quitting_kill_nothing(cx: &mut TestAppContext) {
    let shell = quitting_shell(cx, "quit-test");
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.quit();
    assert_eq!(cx.read(|cx| shell.read(cx).kills), 0);
}

#[gpui::test]
fn stop_all_and_quit_kills_once(cx: &mut TestAppContext) {
    // The guard's "Stop all and quit": the shell kills, then GPUI quits.
    let shell = quitting_shell(cx, "quit-stop-all");
    cx.update(|cx| shell.update(cx, |s, cx| s.stop_all_and_quit(cx)));
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| (shell.read(cx).quits, shell.read(cx).kills)), (1, 1));
    cx.quit();
    assert_eq!(cx.read(|cx| shell.read(cx).kills), 1, "the quit observer doesn't kill again");
}
