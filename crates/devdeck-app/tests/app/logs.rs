//! M3 step 4: real pod logs (R7.5, R7.8, R1.3). The logs view over a scripted driver whose
//! streams the test feeds: first tails merged by time, pods coming and going, streams only
//! while visible and resumed from the last line, waiting and ended containers, refusals,
//! the container and pod picked, a previous run; the buffer's caps. Nothing here reaches a
//! cluster.

use devdeck_app::cluster::{Cluster, ClusterDriver, LOAD_DELAY, LogFrom, LogLine, LogRefusal, LogStream, LogUpdate, PodScope, PodUpdate};
use devdeck_app::shell::Shell;
use devdeck_app::views::logs_feed::{CATCH_UP, DRAIN_EVERY, ENDED_RETRY, Entry, FIRST_LINES, LogsBuffer, MAX_LINES, MERGE_WAIT};
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

/// Pods come from the test (`apply_pods`); each log stream asked is recorded with its
/// sender. `refuse` answers every stream with a refusal.
#[derive(Default)]
struct Scripted {
    asked: RefCell<Vec<Asked>>,
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

    fn end(&self, pod: &str, update: LogUpdate) {
        let asked = self.asked.borrow();
        let a = asked.iter().rev().find(|a| a.pod == pod && !a.previous).expect("a stream");
        a.tx.try_send(update).unwrap();
    }
}

fn pod(id: &str, state: &str, restarts: u32) -> Pod {
    let main = ContainerStatus { name: SVC.into(), init: false, ready: state == "running", restarts, state: state.into(), last_termination: None };
    let init = ContainerStatus { name: "migrate".into(), init: true, ready: false, restarts: 0, state: "terminated: Completed (exit 0)".into(), last_termination: None };
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
    assert_eq!(cx.read(|cx| v.read(cx).matches()), 2);
    t.driver.send(&name("aaaaa"), &[(4.0, "ERROR down once more")]);
    advance(cx, DRAIN_EVERY);
    assert_eq!(cx.read(|cx| v.read(cx).matches()), 3, "a new line that matches counts at once");
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

fn entry(text: &str, level: Level) -> Entry {
    Entry { at: None, pod: "p".into(), text: text.into(), level, note: false }
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
    assert_eq!(b.hits().iter().copied().collect::<Vec<_>>(), vec![4]);
    b.push(entry("line 5 the longest of them all", Level::Error));
    assert_eq!(b.row_count(), 2, "line 2 dropped, line 5 in");
    assert_eq!(b.row(b.longest_row().unwrap()).unwrap().text.as_ref(), "line 5 the longest of them all");
    // Bytes count too.
    let mut small = LogsBuffer::new(100, 10);
    small.push(entry("123456", Level::Info));
    small.push(entry("789012", Level::Info));
    assert_eq!(small.len(), 1, "over the byte budget: the oldest goes");
}
