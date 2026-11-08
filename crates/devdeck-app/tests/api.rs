//! The control API's server (R13), on the demo: the dispatcher with each kind of caller,
//! and one real socket round trip (the test process is an outside caller: it isn't below
//! a devdeck shell).

use devdeck_api::{Caller, Class, ErrorKind, Policy};
use devdeck_app::api::dispatch::{self, CallCx};
use devdeck_app::api::identity::Peer;
use devdeck_app::api::{self, Config, Status, Who};
use devdeck_app::shell::Shell;
use devdeck_core::view_addr::ViewAddr;
use gpui::{Entity, TestAppContext};
use serde_json::{Value, json};

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn terminal(n: u32) -> Who {
    Who::Terminal { workspace: "work".into(), n, uid: "uid-1".into(), shell: 1, starts: 1 }
}

fn outside() -> Who {
    Who::Outside { pid: Some(4242), program: "devdeck-ctl".into() }
}

fn call(who: &Who, method: &str, params: Value, cx: &mut TestAppContext) -> Result<Value, devdeck_api::ApiError> {
    cx.update(|cx| dispatch::call(&CallCx { who, instance: "1-2", state: "demo" }, method, params, cx))
}

#[gpui::test]
fn hello_says_who_calls_and_what_it_may_do(cx: &mut TestAppContext) {
    let _shell = start(cx, "api-hello");
    let v = call(&terminal(3), "session.hello", json!({ "client": { "name": "t", "version": "1" } }), cx).unwrap();
    let hello: devdeck_api::types::Hello = serde_json::from_value(v).unwrap();
    assert_eq!(hello.caller, Caller::Terminal { workspace: "work".into(), n: 3, uid: "uid-1".into() });
    assert_eq!(hello.instance, "1-2");
    let destructive = hello.policy.iter().find(|p| p.class == Class::Destructive).unwrap();
    assert_eq!(destructive.policy, Policy::Ask);
    let e = call(&terminal(3), "session.hello", json!({ "client": { "name": "t", "version": "1" }, "api_level_min": 99 }), cx).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unavailable);
    let e = call(&terminal(3), "session.hello", json!({ "client": { "name": "t", "version": "1" }, "colour": 1 }), cx).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Invalid, "unknown parameters are refused");
}

#[gpui::test]
fn whoami_finds_where_the_terminal_shows(cx: &mut TestAppContext) {
    let shell = start(cx, "api-whoami");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let dir = std::env::temp_dir();
    let tab = ViewAddr::Terminal { n: 7, dir: dir.display().to_string() }.to_ref();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(tab.clone(), l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    let w: devdeck_api::types::Whoami = serde_json::from_value(call(&terminal(7), "session.whoami", json!({}), cx).unwrap()).unwrap();
    assert_eq!(w.workspace.as_deref(), Some("work"));
    assert_eq!(w.terminal.unwrap().n, 7);
    let focused = cx.read(|cx| deck.read(cx).layout().main().focused.0);
    assert_eq!(w.shown.iter().map(|p| p.pane).collect::<Vec<_>>(), vec![focused]);
    let w: devdeck_api::types::Whoami = serde_json::from_value(call(&outside(), "session.whoami", json!({}), cx).unwrap()).unwrap();
    assert_eq!((w.workspace, w.terminal), (None, None));
}

#[gpui::test]
fn outside_callers_reach_only_their_short_list(cx: &mut TestAppContext) {
    let _shell = start(cx, "api-outside");
    assert!(call(&outside(), "devdeck.info", json!({}), cx).is_ok());
    for m in ["run.list", "app.list", "ui.get", "settings.get"] {
        assert_eq!(call(&outside(), m, json!({}), cx).unwrap_err().kind, ErrorKind::PermissionDenied, "{m}");
    }
    let d: devdeck_api::types::Describe = serde_json::from_value(call(&outside(), "api.describe", json!({}), cx).unwrap()).unwrap();
    let allowed: Vec<&str> = d.methods.iter().filter(|m| m.allowed).map(|m| m.name.as_str()).collect();
    assert_eq!(allowed, ["api.describe", "devdeck.info", "session.hello", "session.whoami"]);
    assert!(d.methods.iter().all(|m| m.params.is_none()), "schemas only for one method");
    let one: devdeck_api::types::Describe = serde_json::from_value(call(&terminal(1), "api.describe", json!({ "method": "run.get" }), cx).unwrap()).unwrap();
    assert_eq!(one.methods.len(), 1);
    assert!(one.methods[0].params.is_some() && one.methods[0].allowed);
}

#[gpui::test]
fn the_api_serves_a_socket_and_audits_its_callers(cx: &mut TestAppContext) {
    let _shell = start(cx, "api-socket");
    let rt = cx.update(|cx| devdeck_app::test_dir("api-rt", cx));
    let state_dir = cx.update(|cx| devdeck_app::test_dir("api-state", cx));
    let config = Config { runtime: Some(rt.clone()), state_dir: state_dir.clone(), default_state_dir: state_dir.clone(), audit_file: false };
    let api = cx.update(|cx| api::start(config, cx));
    let path = match cx.read(|cx| api.read(cx).status.clone()) {
        Status::On(p) => p,
        Status::Off(why) => panic!("{why}"),
    };
    assert_eq!(path, rt.join("devdeck/main.sock"));
    assert!(path.exists());
    assert_eq!(cx.read(api::socket_path), Some(path.clone()));
    // What the socket's threads hand over (the transport's own tests use a real socket;
    // GPUI's test scheduler refuses wake-ups from other threads).
    let peer = Peer { pid: Some(std::process::id()), chain: vec![std::process::id()], program: "devdeck-ctl".into() };
    let opened = cx.update(|cx| api::open(&api, &peer, cx));
    assert!(opened.outside, "the test process isn't below a devdeck shell");
    let info = cx.update(|cx| api::call(&api, opened.id, "devdeck.info", json!({}), cx)).unwrap();
    assert_eq!(info["state"], "main");
    let denied = cx.update(|cx| api::call(&api, opened.id, "run.list", json!({}), cx)).unwrap_err();
    assert_eq!(denied.kind, ErrorKind::PermissionDenied);
    let (events, calls) = cx.read(|cx| {
        let s = api.read(cx);
        (s.audit.iter().map(|e| e.event.label()).collect::<Vec<_>>(), s.connections[&opened.id].calls)
    });
    assert_eq!(events, ["denied", "connected"], "newest first");
    assert_eq!(calls, 2);
}

#[gpui::test]
fn a_caller_whose_shell_ended_expires(cx: &mut TestAppContext) {
    let _shell = start(cx, "api-expired");
    let state_dir = cx.update(|cx| devdeck_app::test_dir("api-state", cx));
    let config = Config { runtime: None, state_dir: state_dir.clone(), default_state_dir: state_dir, audit_file: false };
    let api = cx.update(|cx| api::start(config, cx));
    // A terminal caller (as `identify` makes them) whose shell isn't running.
    assert!(!cx.read(|cx| api::alive(&terminal(9), cx)));
    assert!(cx.read(|cx| api::alive(&outside(), cx)));
    let peer = Peer { pid: None, chain: vec![], program: "?".into() };
    let opened = cx.update(|cx| api::open(&api, &peer, cx));
    assert!(cx.update(|cx| api::call(&api, opened.id, "devdeck.info", json!({}), cx)).is_ok(), "an unpinned peer is outside");
}

#[gpui::test]
fn without_a_runtime_folder_the_api_is_off(cx: &mut TestAppContext) {
    let _shell = start(cx, "api-off");
    let state_dir = cx.update(|cx| devdeck_app::test_dir("api-state", cx));
    let config = Config { runtime: None, state_dir: state_dir.clone(), default_state_dir: state_dir, audit_file: false };
    let api = cx.update(|cx| api::start(config, cx));
    assert!(matches!(cx.read(|cx| api.read(cx).status.clone()), Status::Off(why) if why.contains("XDG_RUNTIME_DIR")));
    assert_eq!(cx.read(api::socket_path), None);
}

#[test]
fn no_handler_is_unlisted() {
    assert_eq!(dispatch::handled().1, Vec::<&str>::new());
}
