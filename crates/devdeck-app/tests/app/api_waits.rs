//! The control API's waits and jobs (R13.10), on the demo (fake runs): a wait answers
//! when what it waits for happens, or when its time is up with what it has and a job to
//! wait on; jobs are the caller's own; a caller has a few waits at once.

use devdeck_api::ErrorKind;
use devdeck_api::types::{Jobs, RunState, Waited};
use devdeck_app::api::Who;
use devdeck_app::api::dispatch::{self, Answer, CallCx};
use devdeck_app::api::waits::{TICK, WAITS_PER_CALLER};
use devdeck_app::fake_driver::{LINE_GAP, STOP_DELAY};
use devdeck_app::runner::{RunKey, Runner};
use devdeck_app::shell::Shell;
use gpui::{Entity, TestAppContext};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const APP: &str = "eventservice";
const CMD: &str = "go:run";

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn agent(n: u32) -> Who {
    Who::Terminal { workspace: "work".into(), n, uid: format!("uid-{n}"), shell: n, shell_start: 1, starts: 1 }
}

fn runner(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Runner> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone())
}

type Slot = Rc<RefCell<Option<Result<Value, devdeck_api::ApiError>>>>;

/// Calls `method`; an answer that comes later lands in the slot.
fn ask(who: &Who, method: &str, params: Value, cx: &mut TestAppContext) -> Slot {
    let slot: Slot = Rc::default();
    let answer = cx.update(|cx| dispatch::answer(&CallCx { who, instance: "i", state: "demo", conn: None }, method, params, cx));
    match answer {
        Answer::Now(r) => *slot.borrow_mut() = Some(r),
        Answer::Later(task) => {
            let s = slot.clone();
            cx.update(|cx| cx.spawn(async move |_| *s.borrow_mut() = Some(task.await)).detach());
        }
    }
    cx.run_until_parked();
    slot
}

fn waited(slot: &Slot) -> Option<Result<Waited, devdeck_api::ApiError>> {
    slot.borrow().clone().map(|r| r.map(|v| serde_json::from_value(v).unwrap()))
}

fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

fn started(shell: &Entity<Shell>, cx: &mut TestAppContext) -> Entity<Runner> {
    let r = runner(shell, cx);
    cx.update(|cx| r.update(cx, |r, cx| r.start(&RunKey::new(APP, CMD), None, cx))).unwrap();
    cx.run_until_parked();
    r
}

#[gpui::test]
fn a_wait_answers_when_a_matching_line_comes(cx: &mut TestAppContext) {
    let shell = start(cx, "wait-output");
    started(&shell, cx);
    let slot = ask(&agent(3), "run.wait", json!({"app": APP, "cmd": CMD, "until": {"output": "."}, "timeout": 20}), cx);
    assert!(waited(&slot).is_none(), "nothing written yet");
    advance(cx, LINE_GAP + TICK);
    let w = waited(&slot).expect("answered").unwrap();
    assert!(w.done && w.job.is_none(), "{w:?}");
    assert_eq!(w.state, RunState::Running);
    assert!(!w.output.lines.is_empty(), "what came since it began");
}

#[gpui::test]
fn a_wait_whose_time_is_up_leaves_a_job_to_wait_on(cx: &mut TestAppContext) {
    let shell = start(cx, "wait-job");
    let r = started(&shell, cx);
    let me = agent(3);
    let slot = ask(&me, "run.wait", json!({"app": APP, "cmd": CMD, "until": "exit", "timeout": 1}), cx);
    advance(cx, Duration::from_secs(1) + TICK);
    let w = waited(&slot).expect("time's up").unwrap();
    let job = w.job.clone().expect("a job");
    assert!(!w.done);
    let jobs: Jobs = serde_json::from_value(ask(&me, "job.list", json!({}), cx).borrow().clone().unwrap().unwrap()).unwrap();
    assert_eq!(jobs.jobs.iter().map(|j| (j.id.as_str(), j.what.as_str(), j.done)).collect::<Vec<_>>(), [(job.as_str(), "run eventservice go:run until exit", false)]);
    // Someone else's: not theirs to see.
    let other = waited(&ask(&agent(4), "job.get", json!({"job": job}), cx)).unwrap();
    assert_eq!(other.unwrap_err().kind, ErrorKind::NotFound);
    // Waiting on: it stops meanwhile.
    let again = ask(&me, "job.wait", json!({"job": job, "timeout": 10}), cx);
    cx.update(|cx| r.update(cx, |r, cx| r.stop(&RunKey::new(APP, CMD), cx)));
    advance(cx, STOP_DELAY + TICK * 2);
    let w = waited(&again).expect("it ended").unwrap();
    assert!(w.done && w.job.is_none(), "{w:?}");
    assert_eq!(w.state, RunState::Stopped);
    let cancelled = ask(&me, "job.cancel", json!({"job": job}), cx);
    assert!(cancelled.borrow().clone().unwrap().is_ok());
    let jobs: Jobs = serde_json::from_value(ask(&me, "job.list", json!({}), cx).borrow().clone().unwrap().unwrap()).unwrap();
    assert!(jobs.jobs.is_empty());
}

#[gpui::test]
fn a_caller_has_a_few_waits_at_once(cx: &mut TestAppContext) {
    let shell = start(cx, "wait-cap");
    started(&shell, cx);
    let me = agent(3);
    let waits: Vec<Slot> = (0..WAITS_PER_CALLER).map(|_| ask(&me, "run.wait", json!({"app": APP, "cmd": CMD, "until": "exit", "timeout": 5}), cx)).collect();
    let one_more = waited(&ask(&me, "run.wait", json!({"app": APP, "cmd": CMD, "until": "exit"}), cx)).expect("at once");
    assert_eq!(one_more.unwrap_err().kind, ErrorKind::Busy);
    assert!(waited(&ask(&agent(4), "run.wait", json!({"app": APP, "cmd": CMD, "until": "exit", "timeout": 1}), cx)).is_none(), "another caller's count is its own");
    advance(cx, Duration::from_secs(5) + TICK);
    assert!(waits.iter().all(|w| waited(w).is_some_and(|r| r.is_ok())));
    assert!(waited(&ask(&me, "run.wait", json!({"app": APP, "cmd": CMD, "until": "exit", "timeout": 1}), cx)).is_none(), "ended waits free their place");
}

#[gpui::test]
fn waits_follow_the_reading_rules(cx: &mut TestAppContext) {
    let _shell = start(cx, "wait-rules");
    // The user's terminal holds what they typed: not without their approval.
    let theirs = waited(&ask(&agent(3), "terminal.wait", json!({"n": 1, "until": {"output": "password"}}), cx)).expect("at once");
    assert!(matches!(theirs.unwrap_err().kind, ErrorKind::ApprovalRequired | ErrorKind::NotFound));
    let idle_run = waited(&ask(&agent(3), "run.wait", json!({"app": APP, "cmd": CMD, "until": "idle"}), cx)).expect("at once");
    assert_eq!(idle_run.unwrap_err().kind, ErrorKind::Invalid);
    let bad = waited(&ask(&agent(3), "run.wait", json!({"app": APP, "cmd": CMD, "until": {"output": "("}}), cx)).expect("at once");
    assert!(bad.is_err());
}

/// A one-off read that holds something (a port scan) counts among the caller's waits.
#[gpui::test]
fn one_off_reads_count_among_the_waits(cx: &mut TestAppContext) {
    let shell = start(cx, "wait-reads");
    started(&shell, cx);
    let me = agent(3);
    let waits: Vec<Slot> = (0..WAITS_PER_CALLER).map(|_| ask(&me, "run.wait", json!({"app": APP, "cmd": CMD, "until": "exit", "timeout": 1}), cx)).collect();
    let busy = ask(&me, "port.list", json!({}), cx).borrow().clone().expect("at once");
    assert_eq!(busy.unwrap_err().kind, ErrorKind::Busy);
    advance(cx, Duration::from_secs(1) + TICK);
    assert!(waits.iter().all(|w| waited(w).is_some()));
    let ok = ask(&me, "port.list", json!({}), cx).borrow().clone().expect("the demo answers at once");
    assert!(ok.is_ok(), "the places are free again, the read's too once answered");
    assert!(ask(&me, "port.list", json!({}), cx).borrow().clone().unwrap().is_ok());
}
