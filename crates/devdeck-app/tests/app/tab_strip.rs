//! A tab strip (R3.12, R9.10, R9.11): selecting a tab, or a run in another tab starting,
//! never moves a tab; tabs keep a readable minimum, then the strip scrolls sideways; a tab
//! made active from elsewhere scrolls into view, a wheel scroll stays.

use devdeck_app::deck::Deck;
use devdeck_app::runner::RunKey;
use devdeck_app::shell::Shell;
use devdeck_core::layout::{PaneId, ViewRef};
use devdeck_core::view_addr::ViewAddr;
use gpui::{Bounds, Entity, Modifiers, Pixels, ScrollDelta, ScrollEvent, TestAppContext, VisualTestContext, point};

fn start(cx: &mut TestAppContext, tag: &str) -> (Entity<Shell>, Entity<Deck>) {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    (shell, deck)
}

/// Opens `views` as tabs of the main window's focused pane; returns the pane.
fn open_all(deck: &Entity<Deck>, views: &[ViewRef], cx: &mut TestAppContext) -> PaneId {
    let pane = cx.read(|cx| deck.read(cx).layout().main().focused);
    for v in views {
        let v = v.clone();
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(v, pane).map(|_| ()))));
    }
    cx.run_until_parked();
    pane
}

fn vcx(shell: &Entity<Shell>, cx: &mut TestAppContext) -> VisualTestContext {
    let main = cx.read(|cx| shell.read(cx).main_window().unwrap());
    let v = VisualTestContext::from_window(main.into(), cx);
    v.run_until_parked();
    v
}

fn bounds(v: &mut VisualTestContext, pane: PaneId, n: usize) -> Vec<Option<Bounds<Pixels>>> {
    (0..n).map(|i| v.debug_bounds(Box::leak(format!("tab-{}-{i}", pane.0).into_boxed_str()))).collect()
}

fn activate(deck: &Entity<Deck>, pane: PaneId, i: usize, cx: &mut TestAppContext) {
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.activate(pane, i))));
    cx.run_until_parked();
}

#[gpui::test]
fn selecting_a_tab_or_a_run_starting_moves_no_tab(cx: &mut TestAppContext) {
    let (shell, deck) = start(cx, "tabs-still");
    let views = [
        ViewAddr::Run { app: "eventservice".into(), cmd: "go:run".into() }.to_ref(),
        ViewAddr::App { app: "eventservice".into() }.to_ref(),
        ViewRef::new("demo/notes"),
    ];
    let pane = open_all(&deck, &views, cx);
    let n = cx.read(|cx| deck.read(cx).layout().pane(pane).unwrap().tabs.len());
    let widths = |v: &mut VisualTestContext| bounds(v, pane, n).into_iter().map(|b| b.map(|b| b.size)).collect::<Vec<_>>();
    activate(&deck, pane, 0, cx);
    let sizes = widths(&mut vcx(&shell, cx));
    assert!(sizes.iter().all(Option::is_some));
    for i in 0..n {
        activate(&deck, pane, i, cx);
        assert_eq!(widths(&mut vcx(&shell, cx)), sizes, "tab {i} active: no tab changes size");
    }
    // A click on a tab in view: nothing moves at all (R9.10).
    let mut v = vcx(&shell, cx);
    let strip = v.debug_bounds(Box::leak(format!("pane-tabs-{}", pane.0).into_boxed_str())).expect("the strip");
    let before = bounds(&mut v, pane, n);
    let shown = (0..n).find(|i| before[*i].is_some_and(|b| b.left() >= strip.left() && b.right() <= strip.right() && *i != cx.read(|cx| deck.read(cx).layout().pane(pane).unwrap().active)));
    let i = shown.expect("a tab in view that isn't active");
    v.simulate_click(before[i].unwrap().center(), Modifiers::none());
    v.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().pane(pane).unwrap().active), i, "clicked: active");
    assert_eq!(bounds(&mut vcx(&shell, cx), pane, n), before, "clicked: nothing moved");
    // The run, in its tab, starts: its actions change, no tab moves.
    let runner = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone());
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&RunKey::new("eventservice", "go:run"), None, cx))).unwrap();
    cx.run_until_parked();
    assert_eq!(bounds(&mut vcx(&shell, cx), pane, n), before, "a run started");
}

#[gpui::test]
fn tabs_keep_their_minimum_and_the_strip_scrolls(cx: &mut TestAppContext) {
    let (shell, deck) = start(cx, "tabs-scroll");
    let apps: Vec<String> = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).catalog.apps.iter().map(|a| a.id.clone()).collect());
    let views: Vec<ViewRef> = apps.iter().take(16).map(|a| ViewAddr::App { app: a.clone() }.to_ref()).collect();
    assert!(views.len() >= 12, "enough apps to overflow");
    let pane = open_all(&deck, &views, cx);
    let n = cx.read(|cx| deck.read(cx).layout().pane(pane).unwrap().tabs.len());
    let t = cx.read(|cx| devdeck_ui::theme(cx).clone());
    let mut v = vcx(&shell, cx);
    let strip = v.debug_bounds(Box::leak(format!("pane-tabs-{}", pane.0).into_boxed_str())).expect("the strip");
    let all = bounds(&mut v, pane, n);
    for b in all.iter().flatten() {
        assert!(b.size.width >= t.tab_min_w - logical_px(0.5), "at least the minimum: {b:?}");
    }
    let total: f32 = all.iter().flatten().map(|b| f32::from(b.size.width)).sum();
    assert!(total > f32::from(strip.size.width), "more than the strip holds: it scrolls");
    // Made active from elsewhere (not clicked): it comes into view, the first then the last.
    for i in [0, n - 1] {
        activate(&deck, pane, i, cx);
        let mut v = vcx(&shell, cx);
        let b = bounds(&mut v, pane, n)[i].unwrap();
        assert!(b.right() <= strip.right() + logical_px(0.5) && b.left() >= strip.left() - logical_px(0.5), "tab {i} in view: {b:?} in {strip:?}");
    }
    let mut v = vcx(&shell, cx);
    // A wheel scroll stays through the next renders.
    let first_before = bounds(&mut v, pane, 1)[0].unwrap();
    // At the far right now: the plain (vertical) wheel scrolls it back left.
    v.simulate_event(ScrollEvent { position: strip.center(), delta: ScrollDelta::Pixels(point(logical_px(0.), logical_px(200.))), modifiers: Modifiers::none(), ..Default::default() });
    v.run_until_parked();
    let first_after = bounds(&mut v, pane, 1)[0].unwrap();
    assert_ne!(first_after, first_before, "the wheel scrolled it");
    cx.update(|cx| deck.update(cx, |_, cx| cx.notify()));
    let mut v = vcx(&shell, cx);
    assert_eq!(bounds(&mut v, pane, 1)[0].unwrap(), first_after, "not undone by a render");
}

fn logical_px(x: f32) -> Pixels {
    devdeck_ui::logical(x)
}

/// R9.11: a header offers only its view's own verbs, never a way elsewhere or "start
/// here" (those are the page's links and the tab's menu).
#[gpui::test]
fn headers_offer_only_their_views_own_verbs(cx: &mut TestAppContext) {
    const OWN: [&str; 6] = ["run.run", "run.stop", "run.restart", "run.args", "pane.follow", "view.toggle_rendered"];
    let (shell, deck) = start(cx, "tabs-verbs");
    let services = cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let (env, svc) = cx.read(|cx| {
        let s = services.read(cx);
        (s.env.clone().unwrap_or_default(), s.catalog.apps.iter().find(|a| a.id == "graphservice").map(|a| a.id.clone()).unwrap_or_default())
    });
    let views = [
        ViewAddr::App { app: "eventservice".into() }.to_ref(),
        ViewAddr::Run { app: "eventservice".into(), cmd: "go:run".into() }.to_ref(),
        ViewAddr::Svc { env, name: svc }.to_ref(),
        ViewAddr::Port { port: "5720".into() }.to_ref(),
        ViewAddr::Matrix.to_ref(),
        ViewRef::new("demo/notes"),
    ];
    open_all(&deck, &views, cx);
    cx.executor().advance_clock(devdeck_app::cluster::LOAD_DELAY);
    cx.run_until_parked();
    for v in &views {
        let actions = cx.read(|cx| deck.read(cx).view_actions(v, cx));
        for a in actions {
            assert!(OWN.contains(&a.id), "{}: {} is not its own verb", v.as_str(), a.id);
        }
    }
}
