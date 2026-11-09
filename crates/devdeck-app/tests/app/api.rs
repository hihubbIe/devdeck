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
    Who::Terminal { workspace: "work".into(), n, uid: "uid-1".into(), shell: 1, shell_start: 1, starts: 1 }
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
    let peer = Peer { pid: Some(std::process::id()), chain: vec![(std::process::id(), 1)], program: "devdeck-ctl".into() };
    let opened = cx.update(|cx| api::open(&api, &peer, cx)).expect("opened");
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
fn one_caller_has_a_few_connections_at_most(cx: &mut TestAppContext) {
    let _shell = start(cx, "api-per-caller");
    let state_dir = cx.update(|cx| devdeck_app::test_dir("api-state", cx));
    let config = Config { runtime: None, state_dir: state_dir.clone(), default_state_dir: state_dir, audit_file: false };
    let api = cx.update(|cx| api::start(config, cx));
    let peer = |pid: u32| Peer { pid: Some(pid), chain: vec![], program: "devdeck-ctl".into() };
    for _ in 0..api::CONNECTIONS_PER_CALLER {
        assert!(cx.update(|cx| api::open(&api, &peer(7), cx)).is_some());
    }
    assert!(cx.update(|cx| api::open(&api, &peer(7), cx)).is_none(), "one more from the same caller");
    assert!(cx.update(|cx| api::open(&api, &peer(8), cx)).is_some(), "another caller still gets in");
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
    let opened = cx.update(|cx| api::open(&api, &peer, cx)).expect("opened");
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
fn every_method_has_a_handler_and_no_more() {
    assert_eq!(dispatch::handled(), (Vec::<&str>::new(), Vec::<&str>::new()));
}

fn get<T: serde::de::DeserializeOwned>(method: &str, params: Value, cx: &mut TestAppContext) -> T {
    serde_json::from_value(call(&terminal(1), method, params, cx).unwrap_or_else(|e| panic!("{method}: {e}"))).unwrap()
}

#[gpui::test]
fn a_terminal_sees_its_own_workspace_only(cx: &mut TestAppContext) {
    let _shell = start(cx, "api-scope");
    let w: devdeck_api::types::Workspaces = get("workspace.list", json!({}), cx);
    assert_eq!(w.workspaces.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), ["work"]);
    for m in ["workspace.get", "app.list", "run.list", "terminal.list", "ui.get", "layout.list", "note.list", "env.list", "devdeck.problems"] {
        let e = call(&terminal(1), m, json!({ "workspace": "personal" }), cx).unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotFound, "{m}");
        assert!(call(&terminal(1), m, json!({ "workspace": "work" }), cx).is_ok(), "{m}");
    }
}

#[gpui::test]
fn apps_runs_and_terminals(cx: &mut TestAppContext) {
    let shell = start(cx, "api-runs");
    let apps: devdeck_api::types::Apps = get("app.list", json!({}), cx);
    assert!(!apps.apps.is_empty());
    let names: Vec<String> = apps.apps.iter().map(|a| a.name.0.to_lowercase()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "alphabetical");
    // An app with a command, started.
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let (app_id, cmd_id) = cx.read(|cx| {
        let a = services.read(cx).catalog.apps.iter().find(|a| !a.commands.is_empty()).unwrap().clone();
        (a.id.clone(), a.commands[0].id.clone())
    });
    let runner = cx.read(|cx| services.read(cx).runner.clone());
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&devdeck_app::runner::RunKey::new(&app_id, &cmd_id), None, cx))).unwrap();
    cx.run_until_parked();
    let app: devdeck_api::types::App = get("app.get", json!({ "app": app_id }), cx);
    assert!(app.commands.iter().any(|c| c.id == cmd_id && c.running));
    let runs: devdeck_api::types::Runs = get("run.list", json!({}), cx);
    assert!(runs.runs.iter().any(|r| r.app == app_id && r.cmd == cmd_id && r.state == devdeck_api::types::RunState::Running));
    let run: devdeck_api::types::Run = get("run.get", json!({ "app": app_id, "cmd": cmd_id }), cx);
    assert_eq!((run.state, run.starts), (devdeck_api::types::RunState::Running, 1));
    assert!(run.line.is_some());
    assert_eq!(call(&terminal(1), "run.get", json!({ "app": app_id, "cmd": "nope:x" }), cx).unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(call(&terminal(1), "app.get", json!({ "app": "nope" }), cx).unwrap_err().kind, ErrorKind::NotFound);
    // A terminal tab makes a terminal, with a uid.
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let tab = ViewAddr::Terminal { n: 4, dir: std::env::temp_dir().display().to_string() }.to_ref();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(tab.clone(), l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    let terms: devdeck_api::types::Terminals = get("terminal.list", json!({}), cx);
    let t = terms.terminals.iter().find(|t| t.n == 4).expect("terminal 4");
    assert!(!t.uid.is_empty());
    let one: devdeck_api::types::Terminal = get("terminal.get", json!({ "n": 4 }), cx);
    assert_eq!((one.uid.as_str(), one.shown.len()), (t.uid.as_str(), 1));
    assert!(runs.runs.iter().all(|r| !r.cmd.starts_with("term/")), "terminals aren't runs here");
}

#[gpui::test]
fn the_screen_layouts_notes_envs_and_settings(cx: &mut TestAppContext) {
    let shell = start(cx, "api-ui");
    let ui: devdeck_api::types::Ui = get("ui.get", json!({}), cx);
    assert!(ui.windows[0].main);
    fn tabs(n: &devdeck_api::types::Node, out: &mut Vec<devdeck_api::types::Tab>) {
        match n {
            devdeck_api::types::Node::Split { children, .. } => children.iter().for_each(|c| tabs(&c.node, out)),
            devdeck_api::types::Node::Pane { tabs: t, .. } => out.extend(t.iter().cloned()),
            devdeck_api::types::Node::Unknown => {}
        }
    }
    let mut all = Vec::new();
    tabs(&ui.windows[0].root, &mut all);
    assert!(!all.is_empty() && all.iter().all(|t| t.title.as_ref().is_some_and(|t| !t.0.is_empty())), "titled tabs");
    let layouts: devdeck_api::types::Layouts = get("layout.list", json!({}), cx);
    let current = layouts.layouts.iter().find(|l| l.current).expect("a current layout");
    assert_eq!(current.id, ui.layout);
    let l: devdeck_api::types::Layout = get("layout.get", json!({ "id": ui.layout }), cx);
    assert_eq!(l.windows, ui.windows);
    // A note.
    let notes = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).notes.clone());
    let id = cx.update(|cx| notes.update(cx, |n, cx| n.create("eventservice", "todo", cx))).unwrap();
    let list: devdeck_api::types::Notes = get("note.list", json!({ "app": "eventservice" }), cx);
    assert_eq!(list.notes.iter().map(|n| n.name.0.as_str()).collect::<Vec<_>>(), ["todo"]);
    let n: devdeck_api::types::Note = get("note.get", json!({ "id": id }), cx);
    assert_eq!(n.app, "eventservice");
    // Envs, edit mode, settings.
    let envs: devdeck_api::types::Envs = get("env.list", json!({}), cx);
    assert!(envs.envs.iter().any(|e| e.protected) && envs.envs.iter().filter(|e| e.current).count() <= 1);
    let first = envs.envs[0].name.clone();
    let e: devdeck_api::types::Env = get("env.get", json!({ "name": first }), cx);
    assert_eq!(e.name, first);
    let m: devdeck_api::types::EditMode = get("editmode.get", json!({}), cx);
    assert!(!m.on, "off at every start");
    let s: devdeck_api::types::Settings = get("settings.get", json!({}), cx);
    assert!(s.scale > 0.0 && !s.theme.is_empty());
    let _: devdeck_api::types::Problems = get("devdeck.problems", json!({}), cx);
    let w: devdeck_api::types::Workspace = get("workspace.get", json!({}), cx);
    assert_eq!((w.id.as_str(), w.envs.len()), ("work", envs.envs.len()));
}

#[test]
fn a_parent_devdecks_variables_are_removed_at_start() {
    let mut removed = Vec::new();
    devdeck_app::scrub_env(|n| removed.push(n.to_string()));
    assert_eq!(removed, ["DEVDECK_SOCKET", "DEVDECK_TERMINAL"]);
}
