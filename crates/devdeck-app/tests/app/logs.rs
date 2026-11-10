//! M3 step 4: real pod logs (R7.5, R7.8, R1.3). The logs view over a scripted driver whose
//! streams the test feeds: first tails merged by time, pods coming and going, streams only
//! while visible and resumed from the last line, waiting and ended containers, refusals,
//! the container and pod picked, a previous run; the buffer's caps; following and reading
//! earlier lines back. Nothing here reaches a cluster.

use devdeck_app::cluster::{Cluster, ClusterDriver, LOAD_DELAY, LogFrom, LogLine, LogRefusal, LogStream, LogUpdate, PodScope, PodUpdate};
use devdeck_app::shell::Shell;
use devdeck_app::views::logs_feed::{CATCH_UP, DRAIN_EVERY, ENDED_RETRY, Entry, FIRST_INDEX, FIRST_LINES, Kind, LogsBuffer, MAX_LINES, MERGE_WAIT, cut, match_ranges};
use devdeck_app::views::logs_view::{MinLevel, ServiceLogsView};
use devdeck_core::cluster::{ContainerStatus, Pod, PodPhase};
use devdeck_core::runs::Level;
use devdeck_core::view_addr::ViewAddr;
use gpui::{Context, Entity, SharedString, Task, TestAppContext};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const ENV: &str = "dev35";
const SVC: &str = "graphservice";

/// A stream the view asked for, and the sender that feeds it.
struct Asked {
    pod: String,
    container: String,
    from: LogFrom,
    previous: bool,
    tx: async_channel::Sender<LogUpdate>,
}

/// A read back asked, and the sender that answers it.
struct Read {
    pod: String,
    tx: async_channel::Sender<LogUpdate>,
}

/// Pods come from the test (`apply_pods`); each log stream and read back asked is recorded
/// with its sender. `refuse` answers every stream with a refusal.
#[derive(Default)]
struct Scripted {
    asked: RefCell<Vec<Asked>>,
    reads: RefCell<Vec<Read>>,
    refuse: RefCell<Option<LogRefusal>>,
}

impl ClusterDriver for Scripted {
    fn watch(&self, _env: &str, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        None
    }
    fn namespace(&self, env: &str) -> String {
        env.into()
    }
    fn writes(&self) -> Result<(), String> {
        Err("no".into())
    }
    fn watch_pods(&self, _env: &str, _scope: PodScope, _selector: Option<&str>, _cx: &mut Context<Cluster>) -> Option<Task<()>> {
        Some(Task::ready(()))
    }
    fn stream_logs(&self, _env: &str, pod: &str, container: &str, from: LogFrom, previous: bool, _cx: &mut Context<Cluster>) -> Result<LogStream, String> {
        let (tx, updates) = async_channel::unbounded();
        if let Some(r) = self.refuse.borrow().clone() {
            let _ = tx.try_send(LogUpdate::Refused(r));
        }
        self.asked.borrow_mut().push(Asked { pod: pod.into(), container: container.into(), from, previous, tx });
        Ok(LogStream { updates, keep: Box::new(()) })
    }
    fn read_logs(&self, _env: &str, pod: &str, _container: &str, _cx: &mut Context<Cluster>) -> Result<LogStream, String> {
        let (tx, updates) = async_channel::unbounded();
        self.reads.borrow_mut().push(Read { pod: pod.into(), tx });
        Ok(LogStream { updates, keep: Box::new(()) })
    }
}

impl Scripted {
    /// The live streams asked for `pod` (not previous runs), in order.
    fn of(&self, pod: &str) -> Vec<(String, LogFrom)> {
        self.asked.borrow().iter().filter(|a| a.pod == pod && !a.previous && !a.tx.is_closed()).map(|a| (a.container.clone(), a.from)).collect()
    }

    /// Sends `lines` (time in seconds, text) on the last stream asked for `pod`.
    fn send(&self, pod: &str, lines: &[(f64, &str)]) {
        let asked = self.asked.borrow();
        let a = asked.iter().rev().find(|a| a.pod == pod && !a.previous).expect("a stream");
        let lines = lines.iter().map(|(s, t)| LogLine { at: Some((s * 1e9) as i64), text: t.to_string(), level: devdeck_core::runs::log_level(t) }).collect();
        a.tx.try_send(LogUpdate::Lines(lines)).unwrap();
    }

    /// Read backs asked for `pod`.
    fn reads_of(&self, pod: &str) -> usize {
        self.reads.borrow().iter().filter(|r| r.pod == pod).count()
    }

    /// Answers the last read back of `pod` with `lines` (a time in seconds or none, text),
    /// then `end`.
    fn answer(&self, pod: &str, lines: &[(Option<f64>, &str)], end: LogUpdate) {
        let reads = self.reads.borrow();
        let r = reads.iter().rev().find(|r| r.pod == pod).expect("a read back");
        let lines = lines.iter().map(|(s, t)| LogLine { at: s.map(|s| (s * 1e9) as i64), text: t.to_string(), level: devdeck_core::runs::log_level(t) }).collect();
        let _ = r.tx.try_send(LogUpdate::Lines(lines));
        let _ = r.tx.try_send(end);
    }

    fn end(&self, pod: &str, update: LogUpdate) {
        let asked = self.asked.borrow();
        let a = asked.iter().rev().find(|a| a.pod == pod && !a.previous).expect("a stream");
        a.tx.try_send(update).unwrap();
    }
}

fn pod(id: &str, state: &str, restarts: u32) -> Pod {
    let main = ContainerStatus { name: SVC.into(), init: false, ready: state == "running", restarts, state: state.into(), last_termination: None, ..Default::default() };
    let init = ContainerStatus { name: "migrate".into(), init: true, ready: false, restarts: 0, state: "terminated: Completed (exit 0)".into(), last_termination: None, ..Default::default() };
    Pod {
        name: format!("{SVC}-5d9f-{id}"),
        owner: Some(SVC.into()),
        template: "5d9f".into(),
        phase: if state == "running" { PodPhase::Running } else { PodPhase::Waiting(state.trim_start_matches("waiting: ").into()) },
        ready: state == "running",
        restarts,
        last_termination: None,
        started: Some(1),
        node: "node-1".into(),
        tag: "main".into(),
        containers: vec![main, init],
        unschedulable: None,
    }
}

fn name(id: &str) -> String {
    format!("{SVC}-5d9f-{id}")
}

struct T {
    shell: Entity<Shell>,
    cluster: Entity<Cluster>,
    driver: Rc<Scripted>,
}

fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

/// The demo with its cluster on the scripted driver (the demo's deployment lists kept).
fn start(cx: &mut TestAppContext, tag: &str) -> T {
    let config = cx.update(|cx| devdeck_app::test_dir(tag, cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    let cluster = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).cluster.clone().unwrap());
    cx.update(|cx| cluster.update(cx, |c, cx| c.want(ENV, cx)));
    advance(cx, LOAD_DELAY);
    let driver = Rc::new(Scripted::default());
    let d = driver.clone();
    cx.update(|cx| cluster.update(cx, |c, cx| c.set_driver(c.envs.clone(), d, cx)));
    cx.run_until_parked();
    T { shell, cluster, driver }
}

fn set_pods(t: &T, cx: &mut TestAppContext, pods: Vec<Pod>) {
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.apply_pods(ENV, PodScope::Service(SVC.into()), PodUpdate::Pods(pods), cx)));
    cx.run_until_parked();
}

fn addr() -> ViewAddr {
    ViewAddr::Logs { env: ENV.into(), name: SVC.into() }
}

fn open(t: &T, cx: &mut TestAppContext) -> Entity<ServiceLogsView> {
    let deck = cx.read(|cx| t.shell.read(cx).active_deck());
    let r = addr().to_ref();
    let o = r.clone();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(o, l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    cx.read(|cx| deck.read(cx).view(&r).unwrap().downcast::<ServiceLogsView>().unwrap())
}

/// Another tab of its pane in front, or the logs again.
fn show_logs(t: &T, cx: &mut TestAppContext, shown: bool) {
    let deck = cx.read(|cx| t.shell.read(cx).active_deck());
    let r = addr().to_ref();
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let (pane, at) = l.find_view(&r).unwrap();
                let other = if at == 0 { 1 } else { 0 };
                l.activate(pane, if shown { at } else { other })
            })
        })
    });
    cx.run_until_parked();
}

fn texts(v: &Entity<ServiceLogsView>, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| {
        let b = &v.read(cx).feed.buffer;
        (0..b.row_count()).filter_map(|r| b.row(r)).map(|e| e.text.to_string()).collect()
    })
}

#[gpui::test]
fn first_tails_merge_by_time_then_lines_come_as_they_arrive(cx: &mut TestAppContext) {
    let t = start(cx, "logs-merge");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0), pod("bbbbb", "running", 0)]);
    assert_eq!(t.driver.of(&name("aaaaa")), vec![(SVC.to_string(), LogFrom::Tail(FIRST_LINES))], "the main container, its last lines");
    assert!(cx.read(|cx| v.read(cx).feed.streams()) == 2);
    t.driver.send(&name("aaaaa"), &[(10.0, "a1"), (12.0, "a2")]);
    advance(cx, DRAIN_EVERY);
    assert!(texts(&v, cx).is_empty(), "held until the other pod's first lines");
    t.driver.send(&name("bbbbb"), &[(11.0, "b1"), (13.0, "b2")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(texts(&v, cx), ["a1", "b1", "a2", "b2"]);
    t.driver.send(&name("bbbbb"), &[(14.0, "b3")]);
    t.driver.send(&name("aaaaa"), &[(13.5, "a3")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(texts(&v, cx)[4..], ["b3", "a3"], "later lines in the order they come");
}

#[gpui::test]
fn a_pod_that_never_answers_doesnt_hold_the_others_long(cx: &mut TestAppContext) {
    let t = start(cx, "logs-merge-wait");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0), pod("bbbbb", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "a1")]);
    advance(cx, MERGE_WAIT);
    assert_eq!(texts(&v, cx), ["a1"]);
}

#[gpui::test]
fn pods_join_and_go_during_a_rollout(cx: &mut TestAppContext) {
    let t = start(cx, "logs-rollout");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "old")]);
    advance(cx, DRAIN_EVERY);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0), pod("ccccc", "running", 0)]);
    assert_eq!(t.driver.of(&name("ccccc")), vec![(SVC.to_string(), LogFrom::Tail(CATCH_UP))], "a new pod joins from its start");
    set_pods(&t, cx, vec![pod("ccccc", "running", 0)]);
    let lines = texts(&v, cx);
    assert!(lines.contains(&"pod aaaaa gone".to_string()), "{lines:?}");
    assert!(lines.contains(&"old".to_string()), "its lines stay");
    // Its stream runs to its end.
    t.driver.send(&name("aaaaa"), &[(11.0, "last words")]);
    advance(cx, DRAIN_EVERY);
    assert!(texts(&v, cx).contains(&"last words".to_string()));
    advance(cx, Duration::from_secs(31));
    assert!(cx.read(|cx| v.read(cx).feed.pods().iter().all(|(p, _)| p.as_ref() != name("aaaaa"))), "forgotten after its grace");
}

#[gpui::test]
fn a_forgotten_list_is_not_pods_gone(cx: &mut TestAppContext) {
    let t = start(cx, "logs-forgotten");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    let d = t.driver.clone();
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.set_driver(c.envs.clone(), d, cx)));
    cx.run_until_parked();
    assert!(!texts(&v, cx).iter().any(|l| l.contains("gone")), "another driver forgets pods: nothing went");
}

#[gpui::test]
fn hidden_stops_the_streams_and_shown_resumes_from_the_last_line(cx: &mut TestAppContext) {
    let t = start(cx, "logs-hidden");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "x"), (10.7, "y"), (10.7, "y")]);
    advance(cx, DRAIN_EVERY);
    show_logs(&t, cx, false);
    assert_eq!(cx.read(|cx| v.read(cx).feed.streams()), 0, "nothing runs while hidden (R1.3)");
    assert!(t.driver.of(&name("aaaaa")).is_empty(), "its stream was dropped");
    show_logs(&t, cx, true);
    assert_eq!(t.driver.of(&name("aaaaa")), vec![(SVC.to_string(), LogFrom::Since { at: 10_700_000_000, seen: 2, cap: CATCH_UP })]);
}

#[gpui::test]
fn a_waiting_container_asks_nothing_until_it_runs(cx: &mut TestAppContext) {
    let t = start(cx, "logs-waiting");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "waiting: ImagePullBackOff", 0), pod("bbbbb", "waiting: ImagePullBackOff", 0)]);
    assert!(t.driver.asked.borrow().is_empty(), "no request for a container that can't have logs");
    assert_eq!(cx.read(|cx| v.read(cx).feed.waiting()).get("ImagePullBackOff"), Some(&2));
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0), pod("bbbbb", "waiting: ImagePullBackOff", 0)]);
    assert_eq!(t.driver.of(&name("aaaaa")).len(), 1);
}

#[gpui::test]
fn an_ended_stream_starts_again_when_the_container_restarts(cx: &mut TestAppContext) {
    let t = start(cx, "logs-ended");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "panic: boom")]);
    t.driver.end(&name("aaaaa"), LogUpdate::Ended);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).feed.streams()), 0);
    assert!(cx.read(|cx| v.read(cx).feed.problems()).is_empty(), "an end isn't a problem");
    set_pods(&t, cx, vec![pod("aaaaa", "waiting: CrashLoopBackOff", 1)]);
    assert_eq!(t.driver.of(&name("aaaaa")).len(), 0, "waiting: nothing asked");
    set_pods(&t, cx, vec![pod("aaaaa", "running", 1)]);
    assert_eq!(t.driver.of(&name("aaaaa")), vec![(SVC.to_string(), LogFrom::Since { at: 10_000_000_000, seen: 1, cap: CATCH_UP })]);
}

#[gpui::test]
fn a_target_change_resumes_and_a_refusal_waits_for_it(cx: &mut TestAppContext) {
    let t = start(cx, "logs-refused");
    *t.driver.refuse.borrow_mut() = Some(LogRefusal::Forbidden("your account can't read logs in dev35".into()));
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).feed.refused()).as_deref(), Some("your account can't read logs in dev35"));
    let asked = t.driver.asked.borrow().len();
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    assert_eq!(t.driver.asked.borrow().len(), asked, "not asked again");
    *t.driver.refuse.borrow_mut() = None;
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.restart([ENV.to_string()], cx)));
    cx.run_until_parked();
    assert_eq!(t.driver.of(&name("aaaaa")).len(), 1, "the target changed: asked again");
    assert_eq!(cx.read(|cx| v.read(cx).feed.refused()), None);
}

#[gpui::test]
fn the_container_picked_starts_again_with_its_last_lines(cx: &mut TestAppContext) {
    let t = start(cx, "logs-container");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "main line")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).containers(cx)), vec![(SVC.to_string(), false), ("migrate".to_string(), true)], "init containers too");
    cx.update(|cx| v.update(cx, |v, cx| v.set_container(Some("migrate".into()), cx)));
    cx.run_until_parked();
    assert!(texts(&v, cx).is_empty(), "the lines go");
    assert_eq!(t.driver.of(&name("aaaaa")), vec![("migrate".to_string(), LogFrom::Tail(FIRST_LINES))]);
    // A rollout whose pods don't have it: back to the main one, said once.
    let mut p = pod("bbbbb", "running", 0);
    p.containers.truncate(1);
    set_pods(&t, cx, vec![p]);
    assert_eq!(cx.read(|cx| v.read(cx).feed.container.clone()), None);
    assert!(texts(&v, cx).iter().any(|l| l.contains("isn't in the pods any more")));
}

#[gpui::test]
fn a_pod_picked_shows_its_lines_and_its_previous_run(cx: &mut TestAppContext) {
    let t = start(cx, "logs-pod");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 2), pod("bbbbb", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "from a")]);
    t.driver.send(&name("bbbbb"), &[(11.0, "from b")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).previous_target(cx)), Ok(SharedString::from(name("aaaaa"))), "the only pod that restarted");
    cx.update(|cx| v.update(cx, |v, cx| v.set_pod(Some(name("bbbbb").into()), cx)));
    assert_eq!(texts(&v, cx), ["from b"]);
    assert!(cx.read(|cx| v.read(cx).previous_target(cx)).is_err(), "b hasn't restarted");
    cx.update(|cx| v.update(cx, |v, cx| v.set_pod(Some(name("aaaaa").into()), cx)));
    cx.update(|cx| v.update(cx, |v, cx| v.previous_run(cx)));
    cx.run_until_parked();
    let previous = t.driver.asked.borrow().iter().find(|a| a.previous).map(|a| (a.pod.clone(), a.from));
    assert_eq!(previous, Some((name("aaaaa"), LogFrom::Tail(FIRST_LINES))));
    {
        let asked = t.driver.asked.borrow();
        let a = asked.iter().find(|a| a.previous).unwrap();
        a.tx.try_send(LogUpdate::Lines(vec![LogLine { at: Some(5), text: "panic: before".into(), level: Level::Error }])).unwrap();
        a.tx.try_send(LogUpdate::Ended).unwrap();
    }
    advance(cx, DRAIN_EVERY);
    let lines = texts(&v, cx);
    assert_eq!(lines[1..], ["previous run of aaaaa (graphservice):", "panic: before", "end of the previous run of aaaaa"], "{lines:?}");
}

#[gpui::test]
fn the_view_holds_its_pods_while_open(cx: &mut TestAppContext) {
    let t = start(cx, "logs-interest");
    let before = cx.read(|cx| t.cluster.read(cx).pod_interests());
    let v = open(&t, cx);
    assert_eq!(cx.read(|cx| t.cluster.read(cx).pod_interests()), before + 1);
    let deck = cx.read(|cx| t.shell.read(cx).active_deck());
    let r = addr().to_ref();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&r))));
    drop(v);
    // The window lets go of the view at its next frame.
    cx.update(|cx| cx.refresh_windows());
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| t.cluster.read(cx).pod_interests()), before, "closed: its pods go");
}

#[gpui::test]
fn the_level_filter_and_find_follow_new_lines(cx: &mut TestAppContext) {
    let t = start(cx, "logs-filter");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    cx.update(|cx| v.update(cx, |v, cx| v.set_min_level(MinLevel::Error, cx)));
    t.driver.send(&name("aaaaa"), &[(1.0, r#"{"level":"error","msg":"db down"}"#), (2.0, r#"{"level":30,"msg":"ok"}"#), (3.0, "ERROR db down again")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(texts(&v, cx).len(), 2, "JSON levels read");
    cx.update(|cx| v.update(cx, |v, cx| v.find.update(cx, |f, cx| f.set_text("down", cx))));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| v.read(cx).shown_lines()), 2);
    t.driver.send(&name("aaaaa"), &[(4.0, "ERROR down once more"), (5.0, "ERROR unrelated")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).shown_lines()), 3, "a new line that matches shows at once, the other doesn't");
}

#[gpui::test]
fn a_busy_pods_later_lines_wait_behind_its_first_ones(cx: &mut TestAppContext) {
    let t = start(cx, "logs-merge-busy");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0), pod("bbbbb", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "a tail")]);
    advance(cx, DRAIN_EVERY);
    t.driver.send(&name("aaaaa"), &[(20.0, "a live")]);
    advance(cx, DRAIN_EVERY);
    advance(cx, MERGE_WAIT);
    assert_eq!(texts(&v, cx), ["a tail", "a live"], "a quiet pod doesn't put a pod's live lines before its tail");
}

#[gpui::test]
fn hidden_during_the_merge_shows_what_was_held(cx: &mut TestAppContext) {
    let t = start(cx, "logs-merge-hidden");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0), pod("bbbbb", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "held")]);
    advance(cx, DRAIN_EVERY);
    show_logs(&t, cx, false);
    assert_eq!(texts(&v, cx), ["held"], "a resume skips them: they're shown, not dropped");
    show_logs(&t, cx, true);
    assert_eq!(t.driver.of(&name("aaaaa")), vec![(SVC.to_string(), LogFrom::Since { at: 10_000_000_000, seen: 1, cap: CATCH_UP })]);
}

#[gpui::test]
fn a_stream_that_ends_while_its_container_runs_is_asked_again(cx: &mut TestAppContext) {
    let t = start(cx, "logs-ended-running");
    let _v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "x")]);
    t.driver.end(&name("aaaaa"), LogUpdate::Ended);
    advance(cx, DRAIN_EVERY);
    assert!(t.driver.of(&name("aaaaa")).is_empty());
    advance(cx, ENDED_RETRY);
    assert_eq!(t.driver.of(&name("aaaaa")), vec![(SVC.to_string(), LogFrom::Since { at: 10_000_000_000, seen: 1, cap: CATCH_UP })]);
}

#[gpui::test]
fn not_started_yet_is_asked_again_not_held_for_good(cx: &mut TestAppContext) {
    let t = start(cx, "logs-not-started");
    *t.driver.refuse.borrow_mut() = Some(LogRefusal::Waiting("waiting to start: ContainerCreating".into()));
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).feed.refused()), None, "not a refusal of the logs");
    *t.driver.refuse.borrow_mut() = None;
    advance(cx, ENDED_RETRY);
    assert_eq!(t.driver.of(&name("aaaaa")).len(), 1, "asked again while the pod says it runs");
}

#[gpui::test]
fn the_pod_watch_coming_back_resumes_the_streams(cx: &mut TestAppContext) {
    let t = start(cx, "logs-watch-back");
    let _v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "x")]);
    advance(cx, DRAIN_EVERY);
    let asked = t.driver.asked.borrow().len();
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.apply_pods(ENV, PodScope::Service(SVC.into()), PodUpdate::Problem(Some("can't reach km011gcp (dev35)".into())), cx)));
    cx.run_until_parked();
    assert_eq!(t.driver.asked.borrow().len(), asked, "nothing asked while the watch can't reach the cluster");
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.apply_pods(ENV, PodScope::Service(SVC.into()), PodUpdate::Problem(None), cx)));
    cx.run_until_parked();
    assert_eq!(t.driver.asked.borrow().len(), asked + 1, "a connection that came back: its streams may have died silently");
    assert_eq!(t.driver.of(&name("aaaaa")), vec![(SVC.to_string(), LogFrom::Since { at: 10_000_000_000, seen: 1, cap: CATCH_UP })]);
}

#[gpui::test]
fn a_previous_run_shows_whole_between_its_notes(cx: &mut TestAppContext) {
    let t = start(cx, "logs-previous-whole");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 1)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "live 1")]);
    advance(cx, DRAIN_EVERY);
    cx.update(|cx| v.update(cx, |v, cx| v.previous_run(cx)));
    let previous = || t.driver.asked.borrow().iter().position(|a| a.previous).unwrap();
    let i = previous();
    t.driver.asked.borrow()[i].tx.try_send(LogUpdate::Lines(vec![LogLine { at: Some(1), text: "before 1".into(), level: Level::Plain }])).unwrap();
    t.driver.send(&name("aaaaa"), &[(11.0, "live 2")]);
    advance(cx, DRAIN_EVERY);
    t.driver.asked.borrow()[i].tx.try_send(LogUpdate::Ended).unwrap();
    advance(cx, DRAIN_EVERY);
    assert_eq!(texts(&v, cx), ["live 1", "live 2", "previous run of aaaaa (graphservice):", "before 1", "end of the previous run of aaaaa"]);
}

#[gpui::test]
fn the_pod_rows_logs_open_on_that_pod(cx: &mut TestAppContext) {
    let t = start(cx, "logs-open-for-pod");
    let (services, deck) = cx.read(|cx| {
        let s = t.shell.read(cx);
        let w = s.workspaces.iter().find(|w| w.id == "work").unwrap();
        (w.services.clone(), w.deck.downgrade())
    });
    let ctx = devdeck_app::views::ViewCx { services, deck, view: ViewAddr::Svc { env: ENV.into(), name: SVC.into() }.to_ref() };
    cx.update(|cx| devdeck_app::views::logs_view::open_for_pod(&ctx, addr(), &name("bbbbb"), cx));
    cx.run_until_parked();
    let deck = cx.read(|cx| t.shell.read(cx).active_deck());
    let v = cx.read(|cx| deck.read(cx).view(&addr().to_ref()).unwrap().downcast::<ServiceLogsView>().unwrap());
    assert_eq!(cx.read(|cx| v.read(cx).feed.buffer.pod().cloned()), Some(SharedString::from(name("bbbbb"))));
}

#[gpui::test]
fn lines_past_the_cap_while_paused_are_counted(cx: &mut TestAppContext) {
    let t = start(cx, "logs-paused-cap");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(1.0, "first")]);
    advance(cx, DRAIN_EVERY);
    cx.update(|cx| v.update(cx, |v, cx| v.toggle_pause(cx)));
    let many: Vec<(f64, String)> = (0..MAX_LINES + 5).map(|i| (2.0 + i as f64 * 1e-6, format!("l{i}"))).collect();
    let refs: Vec<(f64, &str)> = many.iter().map(|(a, t)| (*a, t.as_str())).collect();
    t.driver.send(&name("aaaaa"), &refs);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).feed.held()), (MAX_LINES + 5) as u64);
    assert_eq!(texts(&v, cx), ["first"]);
    cx.update(|cx| v.update(cx, |v, cx| v.toggle_pause(cx)));
    assert_eq!(cx.read(|cx| v.read(cx).feed.buffer.len()), MAX_LINES);
    assert_eq!(cx.read(|cx| v.read(cx).feed.held()), 0);
}

#[gpui::test]
fn long_lines_wrap_until_wrapping_is_turned_off(cx: &mut TestAppContext) {
    let t = start(cx, "logs-wrap");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    let long = "x".repeat(2000);
    t.driver.send(&name("aaaaa"), &[(1.0, long.as_str()), (2.0, "short")]);
    advance(cx, DRAIN_EVERY);
    cx.update(|cx| cx.refresh_windows());
    cx.run_until_parked();
    assert!(cx.read(|cx| v.read(cx).wrap), "on by default");
    assert!(cx.read(|cx| v.read(cx).list_rows()) > 2, "the long line takes several rows");
    cx.update(|cx| v.update(cx, |v, cx| v.toggle_wrap(cx)));
    assert_eq!(cx.read(|cx| v.read(cx).list_rows()), 2, "one row a line, scrolling sideways");
}

#[test]
fn matches_are_exact_ranges_ignoring_case() {
    let q: Vec<char> = "down".chars().collect();
    assert_eq!(match_ranges("DB Down, down again; downtown", &q), vec![3..7, 9..13, 21..25], "every match, none overlapping");
    let q: Vec<char> = "élan".chars().collect();
    let text = "un ÉLAN, deux élans";
    let ranges = match_ranges(text, &q);
    assert_eq!(ranges.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), ["ÉLAN", "élan"], "byte ranges on character boundaries");
    assert!(match_ranges("anything", &[]).is_empty());
}

#[gpui::test]
fn the_filter_shows_matching_lines_and_clearing_shows_all(cx: &mut TestAppContext) {
    let t = start(cx, "logs-filter-only");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(1.0, "timeout here, timeout there"), (2.0, "fine"), (3.0, "one more TIMEOUT")]);
    advance(cx, DRAIN_EVERY);
    cx.update(|cx| v.update(cx, |v, cx| v.find.update(cx, |f, cx| f.set_text("timeout", cx))));
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["timeout here, timeout there", "one more TIMEOUT"]);
    cx.update(|cx| v.update(cx, |v, cx| v.find.update(cx, |f, cx| f.set_text("", cx))));
    cx.run_until_parked();
    assert_eq!(texts(&v, cx).len(), 3, "cleared: every line");
}

fn entry(text: &str, level: Level) -> Entry {
    Entry { at: None, pod: "p".into(), text: text.into(), level, kind: Kind::Line }
}

#[test]
fn the_buffer_keeps_its_caps_and_what_it_shows() {
    let mut b = LogsBuffer::new(3, 1 << 20);
    for i in 0..5 {
        b.push(entry(&format!("line {i}"), if i % 2 == 0 { Level::Error } else { Level::Info }));
    }
    assert_eq!((b.len(), b.dropped()), (3, 2));
    assert_eq!(b.row(0).unwrap().text.as_ref(), "line 2");
    b.set_min(MinLevel::Error);
    assert_eq!(b.row_count(), 2, "line 2 and line 4");
    b.set_query("4");
    assert_eq!(b.row_count(), 1, "the filter shows only line 4");
    assert_eq!(b.row(0).unwrap().text.as_ref(), "line 4");
    b.set_query("");
    b.push(entry("line 5 the longest of them all", Level::Error));
    assert_eq!(b.row_count(), 2, "line 2 dropped, line 5 in");
    assert_eq!(b.row(b.longest_row().unwrap()).unwrap().text.as_ref(), "line 5 the longest of them all");
    // Bytes count too.
    let mut small = LogsBuffer::new(100, 10);
    small.push(entry("123456", Level::Info));
    small.push(entry("789012", Level::Info));
    assert_eq!(small.len(), 1, "over the byte budget: the oldest goes");
}

#[test]
fn a_cut_keeps_what_is_older_than_the_oldest_shown() {
    let l = |at: Option<i64>, text: &str| LogLine { at, text: text.into(), level: Level::Plain };
    let read = [l(Some(1), "a"), l(Some(2), "same"), l(None, "between"), l(Some(2), "same"), l(Some(2), "same"), l(Some(3), "b")];
    assert_eq!(cut(&read, (2, 1)), Some(4), "before the group, then 2 of its 3 lines (1 shown), the line between them kept");
    assert_eq!(cut(&read, (2, 3)), Some(1), "all of the group shown");
    assert_eq!(cut(&read, (2, 9)), Some(1), "more shown than read: none of the group");
    assert_eq!(cut(&read, (5, 1)), None, "no line at that time: nothing to line up with");
}

#[test]
fn lines_put_above_go_down_from_the_first_index_within_the_caps() {
    use devdeck_app::views::logs_feed::Prepended;
    let mut b = LogsBuffer::new(5, 1 << 20);
    b.push(entry("c", Level::Info));
    b.push(entry("d", Level::Info));
    assert_eq!(b.first_line(), FIRST_INDEX);
    let added = b.prepend(vec![entry("x1", Level::Info), entry("x2 the longest line", Level::Info), entry("a", Level::Error), entry("b", Level::Info)]);
    assert_eq!(added, Prepended { lines: 3, rows: 3, full: true }, "the newest that fit");
    let all = |b: &LogsBuffer| (0..b.row_count()).filter_map(|r| b.row(r)).map(|e| e.text.to_string()).collect::<Vec<_>>();
    assert_eq!(all(&b), ["x2 the longest line", "a", "b", "c", "d"]);
    assert_eq!((b.first_line(), b.row_line(0)), (FIRST_INDEX - 3, Some(FIRST_INDEX - 3)), "indices going down");
    assert_eq!(b.longest_row(), Some(0));
    assert_eq!(b.dropped(), 0, "nothing pushed out");
    b.set_min(MinLevel::Error);
    assert_eq!(all(&b), ["a"], "the level holds for lines put above");
    b.set_min(MinLevel::All);
    b.push(entry("e", Level::Info));
    assert_eq!((b.len(), b.dropped()), (5, 1), "dropped for room: counted");
    assert_eq!(b.row(b.longest_row().unwrap()).unwrap().text.len(), 1, "the longest went with it");
}

/// One frame of the window.
fn draw(cx: &mut TestAppContext) {
    cx.update(|cx| cx.refresh_windows());
    cx.run_until_parked();
}

/// The `logs.earlier` action.
fn earlier(v: &Entity<ServiceLogsView>, cx: &mut TestAppContext) {
    cx.update(|cx| v.update(cx, |v, cx| v.earlier(cx)));
    cx.run_until_parked();
}

fn top_row(v: &Entity<ServiceLogsView>, cx: &TestAppContext) -> String {
    cx.read(|cx| v.read(cx).top_row())
}

fn follows(v: &Entity<ServiceLogsView>, cx: &TestAppContext) -> bool {
    cx.read(|cx| v.read(cx).follow)
}

/// The wrapped pieces as drawn are those of the rows shown now.
fn assert_pieces_right(v: &Entity<ServiceLogsView>, cx: &TestAppContext) {
    cx.read(|cx| {
        let v = v.read(cx);
        let (cols, pieces) = v.wrapped_pieces().expect("wrapped");
        let b = &v.feed.buffer;
        let mut expected = Vec::new();
        for r in 0..b.row_count() {
            let n = b.row(r).unwrap().text.chars().count().div_ceil(cols).max(1);
            expected.extend((0..n).map(|p| (b.row_line(r).unwrap(), p * cols)));
        }
        assert_eq!(pieces, expected);
    });
}

/// `n` lines of `pod` from `from` seconds, one a second.
fn send_many(t: &T, pod: &str, from: f64, n: usize, cx: &mut TestAppContext) {
    let many: Vec<(f64, String)> = (0..n).map(|i| (from + i as f64, format!("line {i}"))).collect();
    let refs: Vec<(f64, &str)> = many.iter().map(|(a, t)| (*a, t.as_str())).collect();
    t.driver.send(pod, &refs);
    advance(cx, DRAIN_EVERY);
}

#[gpui::test]
fn scrolling_up_stops_following_and_the_bottom_follows_again(cx: &mut TestAppContext) {
    use gpui::{Modifiers, ScrollDelta, ScrollEvent, ScrollSource, TouchPhase, VisualTestContext, point};
    let t = start(cx, "logs-wheel");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    send_many(&t, &name("aaaaa"), 10.0, 300, cx);
    draw(cx);
    let w = cx.read(|cx| cx.windows()[0]);
    let mut vcx = VisualTestContext::from_window(w, cx);
    vcx.run_until_parked();
    let at = vcx.read(|cx| v.read(cx).bounds()).center();
    let wheel = |vcx: &mut VisualTestContext, lines: f32| {
        vcx.simulate_event(ScrollEvent { position: at, delta: ScrollDelta::Lines(point(0., lines)), modifiers: Modifiers::none(), touch_phase: TouchPhase::Moved, source: ScrollSource::Pointer });
        vcx.run_until_parked();
    };
    assert!(follows(&v, cx));
    wheel(&mut vcx, 3.);
    assert!(!follows(&v, cx), "scrolling up stops following");
    wheel(&mut vcx, -10_000.);
    assert!(follows(&v, cx), "down to the bottom: following again");
    wheel(&mut vcx, 10_000.);
    assert!(!follows(&v, cx));
    assert_eq!(t.driver.reads_of(&name("aaaaa")), 0, "reaching the top reads nothing yet");
    wheel(&mut vcx, 1.);
    assert_eq!(t.driver.reads_of(&name("aaaaa")), 1, "scrolling up at the top reads earlier lines");
}

#[gpui::test]
fn earlier_lines_are_cut_at_the_oldest_shown_and_merged_by_time(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0), pod("bbbbb", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "same"), (12.0, "a2")]);
    t.driver.send(&name("bbbbb"), &[(11.0, "b1")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(top_row(&v, cx), "load earlier lines");
    earlier(&v, cx);
    assert!(!follows(&v, cx), "reading back: not following");
    assert_eq!((t.driver.reads_of(&name("aaaaa")), t.driver.reads_of(&name("bbbbb"))), (1, 1), "every pod read once");
    assert_eq!(top_row(&v, cx), "loading earlier lines…");
    let a = [(Some(8.0), "a0"), (Some(10.0), "same"), (Some(10.0), "same"), (Some(12.0), "a2"), (Some(13.0), "a3")];
    t.driver.answer(&name("aaaaa"), &a, LogUpdate::Ended);
    t.driver.answer(&name("bbbbb"), &[(Some(9.0), "b0"), (Some(11.0), "b1")], LogUpdate::Ended);
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["a0", "b0", "same", "same", "b1", "a2"], "both pods' older lines by time, the identical line not shown yet kept");
    assert_eq!(top_row(&v, cx), "start of the logs the cluster serves");
    earlier(&v, cx);
    cx.update(|cx| v.update(cx, |v, cx| v.read_earlier(cx)));
    assert_eq!(t.driver.reads_of(&name("aaaaa")), 1, "at its start: not read again");
    // A restart: its run is new, it can be read again.
    set_pods(&t, cx, vec![pod("aaaaa", "running", 1), pod("bbbbb", "running", 0)]);
    assert_eq!(top_row(&v, cx), "load earlier lines");
}

#[gpui::test]
fn a_second_trigger_while_reading_is_ignored(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier-once");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "x")]);
    advance(cx, DRAIN_EVERY);
    earlier(&v, cx);
    earlier(&v, cx);
    cx.update(|cx| v.update(cx, |v, cx| v.read_earlier(cx)));
    assert_eq!(t.driver.reads_of(&name("aaaaa")), 1);
}

#[gpui::test]
fn rows_in_sight_stay_where_they_were(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier-sight");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    send_many(&t, &name("aaaaa"), 100.0, 300, cx);
    draw(cx);
    earlier(&v, cx);
    draw(cx);
    assert_eq!(cx.read(|cx| v.read(cx).scroll_top()), gpui::Pixels::ZERO, "at the top");
    let long = "x".repeat(2000);
    let read = [(Some(50.0), "e0"), (Some(51.0), "e1"), (Some(52.0), long.as_str()), (Some(53.0), "e3"), (Some(54.0), "e4"), (Some(100.0), "line 0")];
    t.driver.answer(&name("aaaaa"), &read, LogUpdate::Ended);
    cx.run_until_parked();
    let (cols, _) = cx.read(|cx| v.read(cx).wrapped_pieces()).unwrap();
    let rows = 4 + 2000usize.div_ceil(cols);
    let row_h = cx.read(|cx| devdeck_ui::theme(cx).line_height);
    assert_eq!(cx.read(|cx| v.read(cx).scroll_top()), row_h * rows as f32, "moved down by the rows put above, the long line's pieces counted");
    draw(cx);
    assert_eq!(cx.read(|cx| v.read(cx).scroll_top()), row_h * rows as f32, "still after a frame");
    assert_pieces_right(&v, cx);
}

#[gpui::test]
fn a_read_back_is_dropped_when_things_changed_meanwhile(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier-dropped");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "a1")]);
    advance(cx, DRAIN_EVERY);
    let older = [(Some(5.0), "older"), (Some(10.0), "a1")];
    // The env's target changed.
    earlier(&v, cx);
    cx.update(|cx| t.cluster.update(cx, |c, cx| c.restart([ENV.to_string()], cx)));
    cx.run_until_parked();
    t.driver.answer(&name("aaaaa"), &older, LogUpdate::Ended);
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["a1"], "another generation");
    assert_eq!(top_row(&v, cx), "load earlier lines");
    // The container restarted.
    earlier(&v, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 1)]);
    t.driver.answer(&name("aaaaa"), &older, LogUpdate::Ended);
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["a1"], "another run");
    // Hidden meanwhile.
    earlier(&v, cx);
    show_logs(&t, cx, false);
    show_logs(&t, cx, true);
    t.driver.answer(&name("aaaaa"), &older, LogUpdate::Ended);
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["a1"], "hidden");
    // Another container.
    earlier(&v, cx);
    assert_eq!(t.driver.reads_of(&name("aaaaa")), 4);
    cx.update(|cx| v.update(cx, |v, cx| v.set_container(Some("migrate".into()), cx)));
    cx.run_until_parked();
    t.driver.answer(&name("aaaaa"), &older, LogUpdate::Ended);
    cx.run_until_parked();
    assert!(texts(&v, cx).is_empty(), "another container");
}

#[gpui::test]
fn a_read_back_lines_up_with_lines_merging_or_waiting(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier-held");
    let v = open(&t, cx);
    // The merge window open: a's first lines wait for b's.
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0), pod("bbbbb", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "a1")]);
    advance(cx, DRAIN_EVERY);
    assert!(texts(&v, cx).is_empty());
    earlier(&v, cx);
    assert_eq!(t.driver.reads_of(&name("bbbbb")), 0, "nothing of b shown: nothing to line up with");
    t.driver.answer(&name("aaaaa"), &[(Some(5.0), "a0"), (Some(10.0), "a1")], LogUpdate::Ended);
    cx.run_until_parked();
    t.driver.send(&name("bbbbb"), &[(11.0, "b1")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(texts(&v, cx), ["a0", "a1", "b1"]);
    // Paused: a pod that joins has its lines only in the queue.
    let t = start(cx, "logs-earlier-paused");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "a1")]);
    advance(cx, DRAIN_EVERY);
    cx.update(|cx| v.update(cx, |v, cx| v.toggle_pause(cx)));
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0), pod("ccccc", "running", 0)]);
    t.driver.send(&name("ccccc"), &[(20.0, "c1")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).feed.held()), 1);
    earlier(&v, cx);
    t.driver.answer(&name("aaaaa"), &[(Some(5.0), "a0"), (Some(10.0), "a1")], LogUpdate::Ended);
    t.driver.answer(&name("ccccc"), &[(Some(15.0), "c0"), (Some(20.0), "c1")], LogUpdate::Ended);
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["a0", "c0", "a1"]);
    cx.update(|cx| v.update(cx, |v, cx| v.toggle_pause(cx)));
    assert_eq!(texts(&v, cx), ["a0", "c0", "a1", "c1"], "nothing doubled once resumed");
}

#[gpui::test]
fn lines_without_a_time_stay_where_they_are(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier-untimed");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "x")]);
    advance(cx, DRAIN_EVERY);
    earlier(&v, cx);
    t.driver.answer(&name("aaaaa"), &[(None, "banner"), (Some(5.0), "panic: old"), (None, "  at trace"), (Some(10.0), "x")], LogUpdate::Ended);
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["banner", "panic: old", "  at trace", "x"]);
}

#[gpui::test]
fn notes_and_previous_runs_dont_count_as_the_oldest(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier-previous");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 1)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "live")]);
    advance(cx, DRAIN_EVERY);
    cx.update(|cx| v.update(cx, |v, cx| v.previous_run(cx)));
    {
        let asked = t.driver.asked.borrow();
        let a = asked.iter().find(|a| a.previous).unwrap();
        a.tx.try_send(LogUpdate::Lines(vec![LogLine { at: Some(1_000_000_000), text: "before".into(), level: Level::Plain }])).unwrap();
        a.tx.try_send(LogUpdate::Ended).unwrap();
    }
    advance(cx, DRAIN_EVERY);
    earlier(&v, cx);
    t.driver.answer(&name("aaaaa"), &[(Some(5.0), "older"), (Some(10.0), "live")], LogUpdate::Ended);
    cx.run_until_parked();
    assert_eq!(texts(&v, cx)[..2], ["older", "live"], "lined up with the live line, not the previous run's");
    assert_eq!(top_row(&v, cx), "start of the logs the cluster serves");
}

#[gpui::test]
fn a_read_back_that_doesnt_line_up_or_fails_says_so(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier-unaligned");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "x")]);
    advance(cx, DRAIN_EVERY);
    earlier(&v, cx);
    t.driver.answer(&name("aaaaa"), &[(Some(1.0), "boom"), (Some(2.0), "bang")], LogUpdate::Refused(LogRefusal::Other("km011gcp (dev35): stream reset".into())));
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["x"]);
    assert_eq!(top_row(&v, cx), "couldn't read earlier lines: km011gcp (dev35): stream reset");
    earlier(&v, cx);
    assert_eq!(t.driver.reads_of(&name("aaaaa")), 2, "a failed read can be tried again");
    t.driver.answer(&name("aaaaa"), &[(Some(5.0), "a"), (Some(7.0), "b")], LogUpdate::Ended);
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["x"], "nothing to line up with: nothing kept");
    assert_eq!(top_row(&v, cx), "earlier lines of aaaaa don't line up with those shown: none added");
}

#[gpui::test]
fn the_filter_holds_for_lines_read_back(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier-filter");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(10.0, "keep 1"), (11.0, "drop 1")]);
    advance(cx, DRAIN_EVERY);
    cx.update(|cx| v.update(cx, |v, cx| v.find.update(cx, |f, cx| f.set_text("KEEP", cx))));
    cx.run_until_parked();
    earlier(&v, cx);
    t.driver.answer(&name("aaaaa"), &[(Some(5.0), "keep 0"), (Some(6.0), "drop 0"), (Some(10.0), "keep 1")], LogUpdate::Ended);
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["keep 0", "keep 1"]);
    cx.update(|cx| v.update(cx, |v, cx| v.find.update(cx, |f, cx| f.set_text("", cx))));
    cx.run_until_parked();
    assert_eq!(texts(&v, cx), ["keep 0", "drop 0", "keep 1", "drop 1"]);
}

#[gpui::test]
fn a_full_buffer_keeps_what_is_read_and_new_lines_wait_until_following(cx: &mut TestAppContext) {
    let t = start(cx, "logs-earlier-full");
    let v = open(&t, cx);
    set_pods(&t, cx, vec![pod("aaaaa", "running", 0)]);
    t.driver.send(&name("aaaaa"), &[(1_000_000.0, "now")]);
    advance(cx, DRAIN_EVERY);
    earlier(&v, cx);
    let old: Vec<String> = (0..MAX_LINES + 5).map(|i| format!("old {i}")).collect();
    let mut read: Vec<(Option<f64>, &str)> = old.iter().enumerate().map(|(i, t)| (Some(i as f64), t.as_str())).collect();
    read.push((Some(1_000_000.0), "now"));
    t.driver.answer(&name("aaaaa"), &read, LogUpdate::Ended);
    cx.run_until_parked();
    let (len, first, dropped) = cx.read(|cx| {
        let b = &v.read(cx).feed.buffer;
        (b.len(), b.row(0).unwrap().text.to_string(), b.dropped())
    });
    assert_eq!((len, first.as_str(), dropped), (MAX_LINES, "old 6", 0), "the newest that fit, nothing pushed out");
    assert_eq!(top_row(&v, cx), "full: 100 000 lines kept");
    draw(cx);
    assert_pieces_right(&v, cx);
    // Scrolled back and full: new lines wait.
    t.driver.send(&name("aaaaa"), &[(1_000_001.0, "new 1")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).feed.held()), 1, "counted (N new)");
    assert_eq!(texts(&v, cx).last().map(String::as_str), Some("now"), "what's read isn't pushed out");
    cx.update(|cx| v.update(cx, |v, cx| v.set_follow(true, cx)));
    assert_eq!(cx.read(|cx| v.read(cx).feed.held()), 0);
    let (last, first, dropped) = cx.read(|cx| {
        let b = &v.read(cx).feed.buffer;
        (b.row(b.row_count() - 1).unwrap().text.to_string(), b.row(0).unwrap().text.to_string(), b.dropped())
    });
    assert_eq!((last.as_str(), first.as_str(), dropped), ("new 1", "old 7", 1), "following: shown, the oldest dropped for room");
    draw(cx);
    assert_pieces_right(&v, cx);
}
