//! The control API's run and organize methods (R13.1): shells for the caller in a
//! workspace folder (a real one here), the current env, notes, layouts, pins and groups;
//! what the user started, protects or types in asks the user.

use devdeck_api::ErrorKind;
use devdeck_api::types::{GroupPath, Layouts, NoteRevision, Terminal};
use devdeck_app::api::Who;
use devdeck_app::api::dispatch::{self, CallCx};
use devdeck_app::runner::RunKey;
use devdeck_app::shell::{Shell, WorkspaceSpec};
use devdeck_core::config::WorkspaceConfig;
use gpui::{AppContext, Entity, TestAppContext};
use serde_json::{Value, json};

fn agent() -> Who {
    Who::Terminal { workspace: "work".into(), n: 3, uid: "uid-3".into(), shell: 3, shell_start: 1, starts: 1 }
}

fn call<T: serde::de::DeserializeOwned>(method: &str, params: Value, cx: &mut TestAppContext) -> Result<T, devdeck_api::ApiError> {
    let who = agent();
    let v = cx.update(|cx| dispatch::call(&CallCx { who: &who, instance: "i", state: "demo", conn: None }, method, params, cx));
    cx.run_until_parked();
    v.map(|v| serde_json::from_value(v).unwrap())
}

fn demo(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

#[gpui::test]
fn the_callers_shells_open_in_the_background_and_are_its_own(cx: &mut TestAppContext) {
    let base = cx.update(|cx| devdeck_app::test_dir("api-term", cx));
    let web = base.join("projects/web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("Makefile"), "dev:\n\techo dev\n").unwrap();
    let config = cx.update(|cx| devdeck_app::test_dir("api-term-config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let ws = WorkspaceConfig { id: "work".into(), name: "work".into(), folders: vec![base.join("projects")], ..Default::default() };
    let shell = cx.update(|cx| Shell::start(vec![WorkspaceSpec::from_config(ws)], "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    // A fresh workspace shows one empty pane: a tab there would take the keys (that asks).
    // The user opened the app's view first.
    let app_view = devdeck_core::view_addr::ViewAddr::App { app: "web".into() }.to_ref();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(app_view, l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    let focused = cx.read(|cx| deck.read(cx).layout().main().focused);
    let before = cx.read(|cx| deck.read(cx).layout().pane(focused).ok().and_then(|p| p.active_view().cloned()));
    let dir = web.display().to_string();
    // Tests read no machine environment (the tools stay unknown) and start no real shell:
    // the demo's fake driver, the tools known (none).
    let services = cx.read(|cx| shell.read(cx).workspaces[0].services.clone());
    let mut known = (*cx.read(|cx| services.read(cx).catalog.clone())).clone();
    known.tools = devdeck_core::apps::Tools::new(&[]);
    let runner = cx.update(|cx| cx.new(|_| devdeck_app::runner::Runner::new("work".into(), std::rc::Rc::new(known), std::rc::Rc::new(devdeck_app::fake_driver::FakeDriver))));
    cx.update(|cx| services.update(cx, |s, _| s.runner = runner.clone()));
    let t: Terminal = call("terminal.open", json!({"dir": dir}), cx).unwrap();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().pane(focused).ok().and_then(|p| p.active_view().cloned())), before, "its tab behind");
    let starter = cx.read(|cx| runner.read(cx).run(&RunKey::terminal(t.n)).map(|r| r.starter.clone()));
    let pending = cx.read(|cx| runner.read(cx).tools().pending);
    assert!(matches!(starter, Some(devdeck_app::runner::Starter::Caller { .. })), "{starter:?}, tools pending: {pending}");
    assert!(call::<Value>("terminal.output", json!({"n": t.n}), cx).is_ok(), "its output is the caller's to read");
    // A folder outside the workspace's.
    assert_eq!(call::<Terminal>("terminal.open", json!({"dir": "/tmp"}), cx).unwrap_err().kind, ErrorKind::PermissionDenied);
    // Four at most.
    for _ in 0..3 {
        call::<Terminal>("terminal.open", json!({"dir": dir}), cx).unwrap();
    }
    assert_eq!(call::<Terminal>("terminal.open", json!({"dir": dir}), cx).unwrap_err().kind, ErrorKind::Busy);
    // Stopping: by its uid; the user's asks.
    assert_eq!(call::<Terminal>("terminal.stop", json!({"n": t.n, "uid": "nope"}), cx).unwrap_err().kind, ErrorKind::NotFound);
    let uid = cx.read(|cx| runner.read(cx).terminal_uid(&RunKey::terminal(t.n)).unwrap().to_string());
    call::<Terminal>("terminal.stop", json!({"n": t.n, "uid": uid}), cx).unwrap();
    assert!(!cx.read(|cx| deck.read(cx).terminals(cx)).get(&t.n).is_some_and(|x| x.here), "its tab went with it");
    cx.executor().advance_clock(devdeck_app::fake_driver::STOP_DELAY * 2);
    cx.run_until_parked();
    let fifth: Terminal = call("terminal.open", json!({"dir": dir}), cx).expect("one ended: there's room again");
    assert_ne!(fifth.n, t.n);
    cx.update(|cx| deck.update(cx, |d, cx| d.open_terminal(&web, devdeck_app::places::Place::Pane(focused), cx)));
    cx.run_until_parked();
    let users = cx.read(|cx| deck.read(cx).terminals(cx)).into_keys().max().unwrap();
    let uid = cx.read(|cx| runner.read(cx).terminal_uid(&RunKey::terminal(users)).unwrap().to_string());
    assert_eq!(call::<Terminal>("terminal.stop", json!({"n": users, "uid": uid}), cx).unwrap_err().kind, ErrorKind::ApprovalRequired);
}

#[gpui::test]
fn envs_notes_layouts_pins_and_groups(cx: &mut TestAppContext) {
    let shell = demo(cx, "api-organize");
    // Envs: a protected one asks.
    assert!(call::<Value>("env.set_current", json!({"name": "dev32"}), cx).is_ok());
    assert_eq!(call::<Value>("env.set_current", json!({"name": "hades"}), cx).unwrap_err().kind, ErrorKind::ApprovalRequired);
    assert_eq!(call::<Value>("env.set_current", json!({"name": "nope"}), cx).unwrap_err().kind, ErrorKind::NotFound);
    // Notes: revisions, conflicts, the user's typing.
    let n: NoteRevision = call("note.create", json!({"app": "eventservice", "name": "todo"}), cx).unwrap();
    assert_eq!(n.revision, 0);
    let w: NoteRevision = call("note.write", json!({"id": n.id, "text": "a", "base_revision": 0}), cx).unwrap();
    assert_eq!(w.revision, 1);
    assert_eq!(call::<NoteRevision>("note.write", json!({"id": n.id, "text": "b", "base_revision": 0}), cx).unwrap_err().kind, ErrorKind::Conflict);
    let a: NoteRevision = call("note.append", json!({"id": n.id, "text": "\nb"}), cx).unwrap();
    let notes = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).notes.clone());
    assert_eq!(cx.update(|cx| notes.update(cx, |x, _| x.text(&n.id))), "a\nb");
    cx.update(|cx| notes.update(cx, |x, cx| x.set_text(&n.id, "typed".into(), cx)));
    assert_eq!(call::<NoteRevision>("note.write", json!({"id": n.id, "text": "c", "base_revision": a.revision + 1}), cx).unwrap_err().kind, ErrorKind::ApprovalRequired);
    assert_eq!(call::<NoteRevision>("note.create", json!({"name": " "}), cx).unwrap_err().kind, ErrorKind::Invalid);
    assert_eq!(call::<NoteRevision>("note.create", json!({"app": "nope", "name": "x"}), cx).unwrap_err().kind, ErrorKind::NotFound);
    // Layouts.
    let l: Layouts = call("layout.save_as", json!({"name": "triage"}), cx).unwrap();
    let triage = l.layouts.iter().find(|x| x.name.as_str() == "triage").unwrap().id.clone();
    let l: Layouts = call("layout.rename", json!({"id": triage, "name": "Triage"}), cx).unwrap();
    assert!(l.layouts.iter().any(|x| x.name.as_str() == "Triage"));
    // Pins and groups, by list.
    for _ in 0..2 {
        call::<Value>("sidebar.pin", json!({"list": "layouts", "key": triage, "pinned": true}), cx).unwrap();
    }
    assert_eq!(call::<Value>("sidebar.pin", json!({"list": "terminals", "key": "1", "pinned": true}), cx).unwrap_err().kind, ErrorKind::Refused);
    let g: GroupPath = call("group.create", json!({"list": "apps", "name": "back"}), cx).unwrap();
    let inner: GroupPath = call("group.create", json!({"list": "apps", "parent": g.path.as_str(), "name": "api"}), cx).unwrap();
    call::<Value>("group.put", json!({"list": "apps", "node": "eventservice", "group": inner.path.as_str()}), cx).unwrap();
    let moved: GroupPath = call("group.move", json!({"list": "apps", "path": inner.path.as_str()}), cx).unwrap();
    assert_eq!(moved.path.as_str(), "api");
    assert_eq!(call::<Value>("group.rename", json!({"list": "apps", "path": "nope", "name": "x"}), cx).unwrap_err().kind, ErrorKind::NotFound);
    call::<Value>("group.delete", json!({"list": "apps", "path": "api"}), cx).unwrap();
    assert_eq!(call::<Value>("group.put", json!({"list": "notes", "node": "x"}), cx).unwrap_err().kind, ErrorKind::Invalid);
    // Only what the list shows, with names of a sane length.
    assert_eq!(call::<Value>("group.put", json!({"list": "apps", "node": "made-up"}), cx).unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(call::<Value>("sidebar.pin", json!({"list": "apps", "key": "made-up", "pinned": true}), cx).unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(call::<Value>("sidebar.pin", json!({"list": "apps", "key": "x".repeat(300), "pinned": true}), cx).unwrap_err().kind, ErrorKind::Invalid);
    assert!(call::<Value>("sidebar.pin", json!({"list": "apps", "key": "made-up", "pinned": false}), cx).is_ok(), "unpinning what isn't there changes nothing");
}
