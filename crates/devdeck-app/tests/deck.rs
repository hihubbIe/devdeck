//! UI-level tests on GPUI's test platform: OS windows follow the layout model, and
//! view instances are released when their tab closes (leak check, R11.4).

use devdeck_app::deck::Deck;
use devdeck_core::layout::{Drop, MAIN_WINDOW, ViewRef};
use gpui::{AppContext, Entity, TestAppContext};

fn setup(cx: &mut TestAppContext) -> Entity<Deck> {
    cx.update(devdeck_app::init);
    let deck = cx.update(|cx| cx.new(|_| Deck::new(devdeck_app::initial_layout())));
    cx.update(|cx| deck.update(cx, |d, cx| d.sync(cx)));
    cx.run_until_parked();
    deck
}

#[gpui::test]
fn main_window_opens_with_initial_views(cx: &mut TestAppContext) {
    let deck = setup(cx);
    assert_eq!(cx.windows().len(), 1);
    assert_eq!(cx.read(|cx| deck.read(cx).live_views()), 4);
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
    assert_eq!(cx.read(|cx| deck.read(cx).live_views()), 4, "no view lost or duplicated");
}

#[gpui::test]
fn closing_a_detached_window_reattaches_its_tabs(cx: &mut TestAppContext) {
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
    let main_views = cx.read(|cx| {
        let l = &deck.read(cx).layout;
        l.panes_in(MAIN_WINDOW).unwrap().iter().map(|p| l.pane(*p).unwrap().tabs.len()).sum::<usize>()
    });
    assert_eq!(main_views, 4);
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
    assert_eq!(cx.read(|cx| deck.read(cx).live_views()), 1);
    let leaked = cx.update(|cx| deck.update(cx, |d, _| d.leaked_views(3)));
    assert_eq!(leaked, 0, "closed views must be dropped");
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
