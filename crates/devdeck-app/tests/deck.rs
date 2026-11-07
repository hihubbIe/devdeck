//! UI-level tests on GPUI's test platform: OS windows follow the layout model, and
//! view instances are released when their tab closes (leak check, R11.4).

use devdeck_app::deck::Deck;
use devdeck_core::layout::{Drop, MAIN_WINDOW, ViewRef};
use gpui::{AppContext, Entity, TestAppContext};

/// A fresh config folder per test, so tests never touch the real one.
fn config_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("devdeck-apptest-{tag}-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn setup(cx: &mut TestAppContext) -> Entity<Deck> {
    let dir = config_dir("deck");
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let deck = cx.update(|cx| cx.new(|_| Deck::new(devdeck_app::initial_layout())));
    cx.update(|cx| deck.update(cx, |d, cx| d.sync(cx)));
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
            let target = d.layout.main().focused;
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
    let panes = |cx: &TestAppContext| cx.read(|cx| deck.read(cx).layout.panes_in(MAIN_WINDOW).unwrap().len());
    assert_eq!(panes(cx), 3);
    // The focused pane starts as the logs pane, which has two tabs: split it.
    cx.simulate_keystrokes(main, "ctrl-\\");
    cx.run_until_parked();
    assert_eq!(panes(cx), 4, "ctrl-\\ splits the focused pane");
    // Focus followed the moved tab into the new pane; detach it to a window.
    cx.simulate_keystrokes(main, "ctrl-shift-d");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).layout.windows.len()), 2, "ctrl-shift-d detaches");
    assert_eq!(cx.windows().len(), 2);
    assert_eq!(panes(cx), 3, "the emptied pane is removed");
}

#[gpui::test]
fn invalid_operations_report_an_error_instead_of_changing_layout(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let before = cx.read(|cx| deck.read(cx).layout.clone());
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.attach(MAIN_WINDOW, Drop::Into(l.main().focused)))));
    let (after, error) = cx.read(|cx| (deck.read(cx).layout.clone(), deck.read(cx).last_error.clone()));
    assert_eq!(after, before);
    assert!(error.is_some());
}
