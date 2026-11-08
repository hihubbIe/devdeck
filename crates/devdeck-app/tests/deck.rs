//! UI-level tests on GPUI's test platform: OS windows follow the layout model, and
//! view instances are released when their tab closes (leak check, R11.4).

use devdeck_app::deck::Deck;
use devdeck_app::shell::Shell;
use devdeck_core::layout::{Drop, MAIN_WINDOW, ViewRef};
use gpui::{Entity, TestAppContext};

fn setup(cx: &mut TestAppContext) -> Entity<Deck> {
    let dir = cx.update(|cx| devdeck_app::test_dir("deck", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let shell = cx.update(|cx| Shell::start_single(devdeck_app::initial_layout, cx));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.run_until_parked();
    deck
}

#[gpui::test]
fn main_window_opens_with_initial_views(cx: &mut TestAppContext) {
    let deck = setup(cx);
    assert_eq!(cx.windows().len(), 1);
    assert_eq!(cx.read(|cx| deck.read(cx).live_views()), 5);
}

#[gpui::test]
fn detach_opens_a_window_and_reattach_closes_it(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let window = cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let (p, i) = l.find_view(&ViewRef::new("demo/notes")).expect("notes open");
                l.detach(p, i)
            })
        })
    });
    let window = window.expect("detached");
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 2, "a second OS window for the detached tab");

    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            let target = d.layout().main().focused;
            d.apply(cx, |l| l.attach(window, Drop::Into(target)));
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the detached window closes on re-attach");
    assert_eq!(cx.read(|cx| deck.read(cx).live_views()), 5, "no view lost or duplicated");
}

#[gpui::test]
fn closing_a_detached_window_closes_its_tabs(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let window = cx
        .update(|cx| {
            deck.update(cx, |d, cx| {
                d.apply(cx, |l| {
                    let (p, i) = l.find_view(&ViewRef::new("demo/notes")).expect("notes open");
                    l.detach(p, i)
                })
            })
        })
        .expect("detached");
    cx.run_until_parked();
    cx.update(|cx| deck.update(cx, |d, cx| d.close_window(window, cx)));
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert_eq!(cx.read(|cx| deck.read(cx).live_views()), 4, "the detached tab closed with its window");
}

#[gpui::test]
fn reattach_button_path_sends_tabs_back(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let window = cx
        .update(|cx| {
            deck.update(cx, |d, cx| {
                d.apply(cx, |l| {
                    let (p, i) = l.find_view(&ViewRef::new("demo/notes")).expect("notes open");
                    l.detach(p, i)
                })
            })
        })
        .expect("detached");
    cx.run_until_parked();
    cx.update(|cx| deck.update(cx, |d, cx| d.reattach(window, cx)));
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert_eq!(cx.read(|cx| deck.read(cx).live_views()), 5);
}

#[gpui::test]
fn closed_views_are_released(cx: &mut TestAppContext) {
    let deck = setup(cx);
    for v in ["demo/notes", "demo/long", "demo/logs"] {
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&ViewRef::new(v)))));
        cx.run_until_parked();
    }
    // A few more syncs so the closed views age past the check window.
    for _ in 0..3 {
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |_| Ok(()))));
        cx.run_until_parked();
    }
    assert_eq!(cx.read(|cx| deck.read(cx).live_views()), 2);
    let leaked = cx.update(|cx| deck.update(cx, |d, _| d.leaked_views(3)));
    assert_eq!(leaked, 0, "closed views must be dropped");
}

#[gpui::test]
fn keyboard_shortcuts_reach_the_layout(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let main = cx.windows()[0];
    let panes = |cx: &TestAppContext| cx.read(|cx| deck.read(cx).layout().panes_in(MAIN_WINDOW).unwrap().len());
    assert_eq!(panes(cx), 3);
    // The focused pane starts as the logs pane, which has two tabs: split it.
    cx.simulate_keystrokes(main, "ctrl-\\");
    cx.run_until_parked();
    assert_eq!(panes(cx), 4, "ctrl-\\ splits the focused pane");
    // Focus followed the moved tab into the new pane; detach it to a window.
    cx.simulate_keystrokes(main, "ctrl-shift-d");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().windows.len()), 2, "ctrl-shift-d detaches");
    assert_eq!(cx.windows().len(), 2);
    assert_eq!(panes(cx), 3, "the emptied pane is removed");
}

#[gpui::test]
fn invalid_operations_report_an_error_instead_of_changing_layout(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let before = cx.read(|cx| deck.read(cx).layout().clone());
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.attach(MAIN_WINDOW, Drop::Into(l.main().focused)))));
    let (after, error) = cx.read(|cx| (deck.read(cx).layout().clone(), deck.read(cx).last_error.clone()));
    assert_eq!(after, before);
    assert!(error.is_some());
}

// ------------------------------------------------------------------ visibility (engine)

use devdeck_app::deck::DeckEvent;
use std::cell::RefCell;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<(String, bool)>>>;

/// Records every visibility change the deck announces.
fn watch(deck: &Entity<Deck>, cx: &mut TestAppContext) -> Log {
    let log: Log = Rc::default();
    let l = log.clone();
    cx.update(|cx| {
        cx.subscribe(deck, move |_, e: &DeckEvent, _| {
            let DeckEvent::Visibility { shown, hidden } = e;
            l.borrow_mut().extend(shown.iter().map(|r| (r.as_str().to_string(), true)));
            l.borrow_mut().extend(hidden.iter().map(|r| (r.as_str().to_string(), false)));
        })
        .detach()
    });
    log
}

fn take(log: &Log, view: &str) -> Vec<bool> {
    let mut l = log.borrow_mut();
    let out = l.iter().filter(|(v, _)| v == view).map(|(_, s)| *s).collect();
    l.retain(|(v, _)| v != view);
    out
}

fn apply(deck: &Entity<Deck>, cx: &mut TestAppContext, op: impl FnOnce(&mut devdeck_core::layout::Layout) -> devdeck_core::layout::Result<()>) {
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, op)));
    cx.run_until_parked();
}

#[gpui::test]
fn the_deck_announces_each_visibility_change_once(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let log = watch(&deck, cx);
    let (logs_pane, _) = cx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("demo/logs")).unwrap());
    let long_index = cx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("demo/long")).unwrap().1);
    // Another tab in front.
    apply(&deck, cx, |l| l.activate(logs_pane, long_index));
    assert_eq!(take(&log, "demo/long"), [true]);
    assert_eq!(take(&log, "demo/logs"), [false]);
    // Collapsed and expanded.
    apply(&deck, cx, |l| l.toggle_collapse(logs_pane));
    apply(&deck, cx, |l| l.toggle_collapse(logs_pane));
    assert_eq!(take(&log, "demo/long"), [false, true]);
    // Another pane maximized hides this one, restoring shows it.
    let (dep, _) = cx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("matrix")).unwrap());
    apply(&deck, cx, |l| l.toggle_maximize(dep));
    apply(&deck, cx, |l| l.toggle_maximize(dep));
    assert_eq!(take(&log, "demo/long"), [false, true]);
    assert!(take(&log, "matrix").is_empty(), "the maximized pane stays shown");
    assert_eq!(take(&log, "demo/notes"), [false, true], "the other panes too");
    // Sent to another pane, active there: the same instance, still shown, no event.
    let notes = ViewRef::new("demo/notes");
    let instance = cx.read(|cx| deck.read(cx).view(&notes).unwrap().entity_id());
    apply(&deck, cx, |l| l.send_tab(&ViewRef::new("demo/notes"), Some(logs_pane)));
    assert_eq!(cx.read(|cx| deck.read(cx).view(&notes).unwrap().entity_id()), instance);
    assert!(take(&log, "demo/notes").is_empty());
    take(&log, "demo/long");
    // Detached into its own window and re-attached.
    let w = cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let (p, i) = l.find_view(&ViewRef::new("demo/notes")).unwrap();
                l.detach(p, i)
            })
        })
    });
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).is_visible(&notes)), "shown in its window once it is open");
    cx.update(|cx| deck.update(cx, |d, cx| d.reattach(w.unwrap(), cx)));
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).is_visible(&notes)), "and back in the main window");
    log.borrow_mut().clear();
    // Covered by Settings, then shown again: one event each way per view.
    cx.update(|cx| deck.update(cx, |d, cx| d.set_covered(true, cx)));
    assert_eq!(take(&log, "matrix"), [false]);
    cx.update(|cx| deck.update(cx, |d, cx| d.set_covered(true, cx)));
    assert!(take(&log, "matrix").is_empty(), "no event without a change");
    cx.update(|cx| deck.update(cx, |d, cx| d.set_covered(false, cx)));
    assert_eq!(take(&log, "matrix"), [true]);
    // The workspace hidden.
    cx.update(|cx| deck.update(cx, |d, cx| d.set_active(false, cx)));
    assert_eq!(take(&log, "matrix"), [false]);
    cx.update(|cx| deck.update(cx, |d, cx| d.set_active(true, cx)));
    assert_eq!(take(&log, "matrix"), [true]);
    // Closed.
    apply(&deck, cx, |l| l.close_view(&ViewRef::new("matrix")));
    assert_eq!(take(&log, "matrix"), [false]);
}

#[gpui::test]
fn settings_and_layout_switches_drive_visibility_through_the_window(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let main = cx.windows()[0];
    let deployments = ViewRef::new("matrix");
    cx.simulate_keystrokes(main, "ctrl-,");
    cx.run_until_parked();
    assert!(!cx.read(|cx| deck.read(cx).is_visible(&deployments)), "Settings cover the panes");
    cx.simulate_keystrokes(main, "escape");
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).is_visible(&deployments)));
    // Layout b keeps deployments and drops notes; default has notes.
    cx.update(|cx| deck.update(cx, |d, cx| d.toggle_keep(&deployments, cx)));
    cx.update(|cx| deck.update(cx, |d, cx| d.save_layout_as("b", cx))).unwrap();
    apply(&deck, cx, |l| l.close_view(&ViewRef::new("demo/notes")));
    let log = watch(&deck, cx);
    cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(devdeck_core::layout_book::DEFAULT_ID, cx))).unwrap();
    cx.run_until_parked();
    assert_eq!(take(&log, "demo/notes"), [true], "the other layout's tab comes on screen");
    assert!(take(&log, "matrix").is_empty(), "the kept view never left the screen");
}

#[gpui::test]
fn a_tab_dragged_along_its_strip_lands_where_it_is_dropped(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton, VisualTestContext, point};
    let deck = setup(cx);
    let (pane, before) = cx.read(|cx| {
        let l = deck.read(cx).layout();
        let p = l.main().focused;
        (p, l.pane(p).unwrap().tabs.clone())
    });
    assert!(before.len() >= 3, "{before:?}");
    let window = cx.read(|cx| cx.windows()[0]);
    let mut vcx = VisualTestContext::from_window(window, cx);
    // Selectors are 'static: a test can leak its few.
    let tab = |vcx: &mut VisualTestContext, i: usize| vcx.debug_bounds(Box::leak(format!("tab-{}-{i}", pane.0).into_boxed_str())).expect("a tab");
    let (first, third) = (tab(&mut vcx, 0), tab(&mut vcx, 2));
    let target = point(third.center().x - third.size.width * 0.3, third.center().y);
    vcx.simulate_mouse_down(first.center(), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(first.center() + point(devdeck_ui::logical(20.), devdeck_ui::logical(0.)), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    let after = vcx.read(|cx| deck.read(cx).layout().pane(pane).unwrap().tabs.clone());
    let mut expected = before.clone();
    let moved = expected.remove(0);
    expected.insert(1, moved);
    assert_eq!(after, expected, "before the third tab (its left half), not at the end");
}
