//! The control API's present methods (R13.1), on the demo: views, panes and layouts change
//! without moving the user's keys; what would take them asks the user; panes are named
//! with the live layout; toasts, settings and the rate limit.

use devdeck_api::ErrorKind;
use devdeck_api::types::{Notified, Toasts, Ui};
use devdeck_app::api::Who;
use devdeck_app::api::dispatch::{self, Answer, CallCx};
use devdeck_app::deck::Deck;
use devdeck_app::shell::Shell;
use devdeck_core::layout::{MAIN_WINDOW, PaneId, ViewRef};
use gpui::{Entity, TestAppContext};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

fn start(cx: &mut TestAppContext, tag: &str) -> (Entity<Shell>, Entity<Deck>) {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    // As `api::start` does: which workspace each toast showed in.
    cx.update(devdeck_app::api::present::follow_toasts);
    cx.run_until_parked();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    (shell, deck)
}

fn agent() -> Who {
    Who::Terminal { workspace: "work".into(), n: 3, uid: "uid-3".into(), shell: 3, shell_start: 1, starts: 1 }
}

fn call<T: serde::de::DeserializeOwned>(method: &str, params: Value, cx: &mut TestAppContext) -> Result<T, devdeck_api::ApiError> {
    let who = agent();
    let slot: Rc<RefCell<Option<Result<Value, devdeck_api::ApiError>>>> = Rc::default();
    match cx.update(|cx| dispatch::answer(&CallCx { who: &who, instance: "i", state: "demo", conn: None }, method, params, cx)) {
        Answer::Now(r) => *slot.borrow_mut() = Some(r),
        Answer::Later(task) => {
            let s = slot.clone();
            cx.update(|cx| cx.spawn(async move |_| *s.borrow_mut() = Some(task.await)).detach());
        }
    }
    cx.run_until_parked();
    let r = slot.borrow_mut().take().expect("answered");
    r.map(|v| serde_json::from_value(v).unwrap())
}

fn layout_id(deck: &Entity<Deck>, cx: &TestAppContext) -> String {
    cx.read(|cx| deck.read(cx).book.current.clone())
}

/// Gives the keys to the demo's logs view (the one that takes keys); its pane.
fn give_keys(deck: &Entity<Deck>, cx: &mut TestAppContext) -> PaneId {
    let logs = ViewRef::new("demo/logs");
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| {
        let (p, i) = l.find_view(&logs).unwrap();
        l.activate(p, i)
    })));
    cx.run_until_parked();
    let pane = cx.read(|cx| deck.read(cx).layout().main().focused);
    let handle = cx.read(|cx| deck.read(cx).view_focus(pane)).unwrap();
    let main = cx.read(|cx| cx.windows()[0]);
    cx.update(|cx| main.update(cx, |_, window, cx| handle.focus(window, cx))).unwrap();
    cx.run_until_parked();
    pane
}

fn active(deck: &Entity<Deck>, pane: PaneId, cx: &TestAppContext) -> Option<ViewRef> {
    cx.read(|cx| deck.read(cx).layout().pane(pane).ok().and_then(|p| p.active_view().cloned()))
}

#[gpui::test]
fn views_open_in_the_background_and_never_take_the_keys(cx: &mut TestAppContext) {
    let (_shell, deck) = start(cx, "present-open");
    let keys = give_keys(&deck, cx);
    let app = json!({"kind": "app", "app": "eventservice"});
    let _: Ui = call("view.open", json!({"view": app}), cx).unwrap();
    let r = ViewRef::new("app/eventservice");
    assert_eq!(cx.read(|cx| deck.read(cx).layout().find_view(&r).map(|(p, _)| p)), Some(keys), "into the focused pane");
    assert_eq!(active(&deck, keys, cx), Some(ViewRef::new("demo/logs")), "behind the tab the user types in");
    // Shown in the pane holding the keys: that asks.
    let err = call::<Ui>("view.open", json!({"view": app, "activate": true}), cx).unwrap_err();
    assert_eq!(err.kind, ErrorKind::ApprovalRequired);
    // Shown elsewhere: fine.
    let layout = layout_id(&deck, cx);
    let other = cx.read(|cx| deck.read(cx).layout().panes_in(MAIN_WINDOW).unwrap().into_iter().find(|p| *p != keys).unwrap());
    let _: Ui = call("view.open", json!({"view": {"kind": "app", "app": "graphservice"}, "pulse": layout, "pane": other.0, "activate": true}), cx).unwrap();
    assert_eq!(active(&deck, other, cx), Some(ViewRef::new("app/graphservice")));
    assert_eq!(cx.read(|cx| deck.read(cx).layout().main().focused), keys, "the focused pane stayed");
    // Things the workspace hasn't: no junk tabs.
    assert_eq!(call::<Ui>("view.open", json!({"view": {"kind": "app", "app": "nope"}}), cx).unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(call::<Ui>("view.open", json!({"view": {"kind": "matrix"}}), cx).unwrap_err().kind, ErrorKind::Invalid);
    let outside = call::<Ui>("file.open", json!({"path": "/etc/hostname"}), cx).unwrap_err();
    assert!(matches!(outside.kind, ErrorKind::PermissionDenied | ErrorKind::NotFound), "{outside:?}");
}

#[gpui::test]
fn panes_are_named_with_the_live_layout_and_states_are_asked_for(cx: &mut TestAppContext) {
    let (_shell, deck) = start(cx, "present-panes");
    let layout = layout_id(&deck, cx);
    let pane = cx.read(|cx| deck.read(cx).layout().panes_in(MAIN_WINDOW).unwrap()[1]);
    let stale = call::<Ui>("pane.collapse", json!({"pulse": "elsewhere", "pane": pane.0, "collapsed": true}), cx).unwrap_err();
    assert_eq!(stale.kind, ErrorKind::Conflict);
    for _ in 0..2 {
        let _: Ui = call("pane.collapse", json!({"pulse": layout, "pane": pane.0, "collapsed": true}), cx).unwrap();
        assert!(cx.read(|cx| deck.read(cx).layout().pane(pane).unwrap().collapsed), "asking twice keeps it collapsed");
    }
    let main = call::<Ui>("window.close", json!({"pulse": layout, "window": MAIN_WINDOW.0}), cx).unwrap_err();
    assert_eq!(main.kind, ErrorKind::Invalid, "closing the main window quits: not through the API");
}

#[gpui::test]
fn closing_a_stopped_terminals_tab_asks(cx: &mut TestAppContext) {
    let (shell, deck) = start(cx, "present-term");
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let dir = cx.read(|cx| deck.read(cx).terminal_folder(None, cx));
    let focused = cx.read(|cx| deck.read(cx).layout().main().focused);
    cx.update(|cx| deck.update(cx, |d, cx| d.open_terminal(&dir, devdeck_app::places::Place::Pane(focused), cx)));
    cx.run_until_parked();
    let (n, view) = cx.read(|cx| deck.read(cx).terminals(cx)).into_iter().map(|(n, t)| (n, t.view)).max_by_key(|(n, _)| *n).unwrap();
    let runner = cx.read(|cx| services.read(cx).runner.clone());
    cx.update(|cx| runner.update(cx, |r, cx| r.stop(&devdeck_app::runner::RunKey::terminal(n), cx)));
    cx.executor().advance_clock(devdeck_app::fake_driver::STOP_DELAY * 2);
    cx.run_until_parked();
    let dir = cx.read(|cx| deck.read(cx).terminal_folder(None, cx)).display().to_string();
    let err = call::<Ui>("view.close", json!({"view": {"kind": "terminal", "n": n, "dir": dir}}), cx).unwrap_err();
    assert_eq!(err.kind, ErrorKind::ApprovalRequired, "{err:?}");
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&view).is_some()), "its tab stays");
}

#[gpui::test]
fn a_layout_switch_asks_while_the_user_types(cx: &mut TestAppContext) {
    let (_shell, deck) = start(cx, "present-switch");
    give_keys(&deck, cx);
    // Saving as makes the copy current: the one to switch to is the default one.
    cx.update(|cx| deck.update(cx, |d, cx| d.save_layout_as("other", cx))).unwrap();
    let err = call::<Ui>("pulse.switch", json!({"id": devdeck_core::layout_book::DEFAULT_ID}), cx).unwrap_err();
    assert_eq!(err.kind, ErrorKind::ApprovalRequired);
    assert_eq!(call::<Ui>("pulse.switch", json!({"id": "nope"}), cx).unwrap_err().kind, ErrorKind::NotFound);
}

#[gpui::test]
fn toasts_settings_and_the_rate_limit(cx: &mut TestAppContext) {
    let (_shell, _deck) = start(cx, "present-misc");
    let n: Notified = call("ui.notify", json!({"text": "built\n\u{202e}ok", "kind": "info"}), cx).unwrap();
    let toasts: Toasts = call("ui.toasts", json!({}), cx).unwrap();
    let t = toasts.toasts.iter().find(|t| t.id == n.id).unwrap();
    assert_eq!(t.text.as_str(), "work · terminal 3: built ok", "labelled, one clean line");
    assert_eq!(call::<Notified>("ui.notify", json!({"text": "again", "kind": "warn"}), cx).unwrap_err().kind, ErrorKind::RateLimited);
    assert_eq!(call::<Value>("settings.set", json!({"theme": "nope"}), cx).unwrap_err().kind, ErrorKind::NotFound);
}

#[gpui::test]
fn a_caller_changes_things_at_a_bounded_rate(cx: &mut TestAppContext) {
    let (_shell, deck) = start(cx, "present-rate");
    let layout = layout_id(&deck, cx);
    let pane = cx.read(|cx| deck.read(cx).layout().main().focused);
    let limited = (0..devdeck_app::api::dispatch::CALLS_PER_WINDOW + 1)
        .map(|_| call::<Ui>("pane.collapse", json!({"pulse": layout, "pane": pane.0, "collapsed": false}), cx))
        .filter(|r| r.as_ref().is_err_and(|e| e.kind == ErrorKind::RateLimited))
        .count();
    assert_eq!(limited, 1, "the call past the rate");
}
