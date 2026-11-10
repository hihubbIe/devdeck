//! The control API's events (R13.10), on the demo (fake runs): a connection follows
//! topics and hears what changed in its own workspace, gathered every 100 ms, without
//! the content; nothing is watched while nobody follows anything.

use devdeck_app::api::dispatch::{self, CallCx};
use devdeck_app::api::events::EVERY;
use devdeck_app::api::{self, ApiState, Config, Who};
use devdeck_app::runner::RunKey;
use devdeck_app::shell::Shell;
use gpui::{Entity, TestAppContext};
use serde_json::{Value, json};
use std::sync::mpsc::{Receiver, sync_channel};

fn start(cx: &mut TestAppContext, tag: &str) -> (Entity<Shell>, Entity<ApiState>) {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    let state_dir = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-state"), cx));
    let api = cx.update(|cx| api::start(Config { runtime: api::no_runtime(), state_dir: state_dir.clone(), default_state_dir: state_dir, audit_file: false }, cx));
    // The demo's shells have no process: these callers are there unless a test says.
    cx.update(|cx| api.update(cx, |s, _| s.alive = |who, _| !matches!(who, Who::Terminal { n: 99, .. })));
    cx.run_until_parked();
    (shell, api)
}

fn agent(workspace: &str, n: u32) -> Who {
    Who::Terminal { workspace: workspace.into(), n, uid: format!("uid-{n}"), shell: n, shell_start: 1, starts: 1 }
}

/// A connection of `who` taking events, and what it receives.
fn connect(api: &Entity<ApiState>, who: &Who, cx: &mut TestAppContext) -> (u64, Receiver<Vec<u8>>) {
    let (tx, rx) = sync_channel(64);
    let opened = cx.update(|cx| api.update(cx, |s, cx| s.open(who.clone(), None, Some(tx), cx))).expect("opened");
    (opened.id, rx)
}

fn call(who: &Who, conn: u64, method: &str, params: Value, cx: &mut TestAppContext) -> Result<Value, devdeck_api::ApiError> {
    cx.update(|cx| dispatch::call(&CallCx { who, instance: "i", state: "demo", conn: Some(conn) }, method, params, cx))
}

/// The events received, as (topic, data).
fn events(rx: &Receiver<Vec<u8>>) -> Vec<(String, Value)> {
    rx.try_iter()
        .map(|l| serde_json::from_slice::<Value>(&l).unwrap())
        .filter(|m| m["method"] == "event")
        .map(|m| (m["params"]["topic"].as_str().unwrap().to_string(), m["params"]["data"].clone()))
        .collect()
}

fn tick(cx: &mut TestAppContext) {
    cx.executor().advance_clock(EVERY * 2);
    cx.run_until_parked();
}

#[gpui::test]
fn a_subscriber_hears_its_workspaces_changes_and_no_content(cx: &mut TestAppContext) {
    let (shell, api) = start(cx, "events-runs");
    let me = agent("work", 3);
    let (conn, rx) = connect(&api, &me, cx);
    let elsewhere = agent("personal", 1);
    let (other, other_rx) = connect(&api, &elsewhere, cx);
    assert!(!cx.read(|cx| api.read(cx).watching()), "nothing watched yet");
    let topics = call(&me, conn, "events.subscribe", json!({"topics": ["runs", "output"]}), cx).unwrap();
    assert_eq!(topics["topics"], json!(["runs", "output"]));
    call(&elsewhere, other, "events.subscribe", json!({"topics": ["runs"]}), cx).unwrap();
    assert!(cx.read(|cx| api.read(cx).watching()));
    let runner = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone());
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&RunKey::new("eventservice", "go:run"), None, cx))).unwrap();
    tick(cx);
    cx.executor().advance_clock(devdeck_app::fake_driver::LINE_GAP * 3);
    tick(cx);
    let got = events(&rx);
    assert!(got.contains(&("runs".into(), json!({"app": "eventservice", "cmd": "go:run", "state": "running", "exit_code": null}))), "{got:?}");
    // The demo runs other servers too: this run's output, gathered (one per send at most).
    let outputs: Vec<_> = got.iter().filter(|(t, d)| t == "output" && d["app"] == "eventservice").collect();
    assert!(!outputs.is_empty() && outputs.len() <= 4, "gathered, not one per line: {outputs:?}");
    assert!(outputs.iter().all(|(_, d)| *d == json!({"app": "eventservice", "cmd": "go:run"})), "what changed, not the text");
    assert!(events(&other_rx).is_empty(), "another workspace's caller hears nothing of this one");
    // Nobody follows anything: nothing is watched.
    call(&me, conn, "events.unsubscribe", json!({"topics": ["runs", "output"]}), cx).unwrap();
    call(&elsewhere, other, "events.unsubscribe", json!({"topics": ["runs"]}), cx).unwrap();
    assert!(!cx.read(|cx| api.read(cx).watching()));
}

#[gpui::test]
fn notes_and_toasts_say_which_changed(cx: &mut TestAppContext) {
    let (shell, api) = start(cx, "events-notes");
    let me = agent("work", 3);
    let (conn, rx) = connect(&api, &me, cx);
    call(&me, conn, "events.subscribe", json!({"topics": ["notes", "toasts", "layout"]}), cx).unwrap();
    let notes = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).notes.clone());
    let id = cx.update(|cx| notes.update(cx, |n, cx| n.create("eventservice", "todo", cx))).unwrap();
    cx.update(|cx| notes.update(cx, |n, cx| n.write(&id, "x".into(), cx))).unwrap();
    let toast = cx.update(|cx| devdeck_ui::toasts(cx).update(cx, |t, cx| t.push(devdeck_ui::ToastKind::Info, "hello", vec![], cx)));
    tick(cx);
    let got = events(&rx);
    assert!(got.contains(&("notes".into(), json!({"id": id, "revision": 1}))), "{got:?}");
    assert!(got.contains(&("toasts".into(), json!({"id": toast, "kind": "info"}))), "{got:?}");
    assert!(call(&me, conn, "events.subscribe", json!({"topics": ["what"]}), cx).is_err(), "an unknown topic");
}

#[gpui::test]
fn a_terminals_output_reaches_that_terminal_only(cx: &mut TestAppContext) {
    let (shell, api) = start(cx, "events-own");
    let (me, other) = (agent("work", 3), agent("work", 4));
    let (conn, rx) = connect(&api, &me, cx);
    let (conn2, rx2) = connect(&api, &other, cx);
    for (who, c) in [(&me, conn), (&other, conn2)] {
        call(who, c, "events.subscribe", json!({"topics": ["output"]}), cx).unwrap();
    }
    let runner = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone());
    cx.update(|cx| runner.update(cx, |_, cx| cx.emit(devdeck_app::runner::RunnerEvent::Output { key: RunKey::terminal(3) })));
    tick(cx);
    assert!(events(&rx).contains(&("output".into(), json!({"n": 3}))));
    assert!(!events(&rx2).iter().any(|(_, d)| d.get("n").is_some()), "not terminal 3's activity");
}

#[gpui::test]
fn closing_or_an_ended_shell_stops_the_watching(cx: &mut TestAppContext) {
    let (shell, api) = start(cx, "events-end");
    let me = agent("work", 3);
    let (conn, _rx) = connect(&api, &me, cx);
    call(&me, conn, "events.subscribe", json!({"topics": ["layout"]}), cx).unwrap();
    assert!(cx.read(|cx| api.read(cx).watching()));
    cx.update(|cx| api.update(cx, |s, cx| s.close_connection(conn, cx)));
    assert!(!cx.read(|cx| api.read(cx).watching()), "its connection closed");
    // A caller whose shell ended (99 here) follows nothing once something changes.
    let gone = agent("work", 99);
    let (conn, rx) = connect(&api, &gone, cx);
    call(&gone, conn, "events.subscribe", json!({"topics": ["layout"]}), cx).unwrap();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.update(|cx| deck.update(cx, |_, cx| cx.notify()));
    tick(cx);
    assert!(events(&rx).is_empty());
    assert!(!cx.read(|cx| api.read(cx).watching()));
}
