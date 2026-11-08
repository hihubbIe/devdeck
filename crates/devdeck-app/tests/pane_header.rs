//! A pane with one tab, or none, has a header instead of a tab strip (R3.12).

use devdeck_app::deck::Deck;
use devdeck_app::shell::Shell;
use devdeck_core::layout::{Drop, Layout, PaneId, ViewRef};
use devdeck_ui::{MenuItem, logical};
use gpui::{AnyWindowHandle, Bounds, Entity, Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext, point, size};

fn setup(cx: &mut TestAppContext, layout: fn() -> Layout) -> Entity<Deck> {
    let dir = cx.update(|cx| devdeck_app::test_dir("pane-header", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let shell = cx.update(|cx| Shell::start_single(layout, cx));
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.run_until_parked();
    deck
}

/// The main window holding one pane with one tab.
fn one_tab() -> Layout {
    let mut l = Layout::new();
    let a = l.main().focused;
    l.open(ViewRef::new("demo/notes"), a).expect("open notes");
    l
}

fn apply(deck: &Entity<Deck>, cx: &mut TestAppContext, op: impl FnOnce(&mut Layout) -> devdeck_core::layout::Result<()>) {
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, op)));
    cx.run_until_parked();
}

fn pane_of(deck: &Entity<Deck>, view: &str, cx: &mut TestAppContext) -> PaneId {
    cx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new(view)).expect("open").0)
}

fn tabs(deck: &Entity<Deck>, pane: PaneId, cx: &mut TestAppContext) -> Vec<ViewRef> {
    cx.read(|cx| deck.read(cx).layout().pane(pane).map(|p| p.tabs.clone()).unwrap_or_default())
}

fn window(cx: &mut TestAppContext, i: usize) -> VisualTestContext {
    let w = cx.read(|cx| cx.windows()[i]);
    let vcx = VisualTestContext::from_window(w, cx);
    vcx.run_until_parked();
    vcx
}

/// Selectors are 'static: a test can leak its few.
fn at(vcx: &mut VisualTestContext, what: &str, pane: PaneId) -> Option<Bounds<Pixels>> {
    let name = if what == "tab" { format!("tab-{}-0", pane.0) } else { format!("pane-{what}-{}", pane.0) };
    vcx.debug_bounds(Box::leak(name.into_boxed_str()))
}

fn menu(vcx: &mut VisualTestContext, w: AnyWindowHandle) -> Vec<String> {
    let menu = vcx.update(|_, cx| devdeck_ui::overlay::layer(w, cx).read(cx).menu.clone()).expect("a menu");
    vcx.read(|cx| menu.read(cx).items().iter().map(|i: &MenuItem| i.label.to_string()).collect())
}

fn right_click(vcx: &mut VisualTestContext, p: gpui::Point<Pixels>) {
    vcx.simulate_mouse_down(p, MouseButton::Right, Modifiers::none());
    vcx.run_until_parked();
}

fn drag(vcx: &mut VisualTestContext, from: gpui::Point<Pixels>, to: gpui::Point<Pixels>) {
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(from + point(logical(20.), logical(0.)), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn one_tab_shows_a_header_more_the_strip_and_the_body_never_moves(cx: &mut TestAppContext) {
    let deck = setup(cx, devdeck_app::initial_layout);
    let b = pane_of(&deck, "matrix", cx);
    let a = pane_of(&deck, "demo/logs", cx);
    let mut vcx = window(cx, 0);
    assert!(at(&mut vcx, "header", b).is_some(), "one tab: the header");
    assert!(at(&mut vcx, "tabs", b).is_none(), "no tab list");
    assert!(at(&mut vcx, "close", b).is_some(), "its ×");
    assert!(at(&mut vcx, "header", a).is_none() && at(&mut vcx, "tabs", a).is_some(), "three tabs: the strip");
    let top = at(&mut vcx, "body", b).expect("the body").origin.y;
    let title = at(&mut vcx, "tab", b).expect("the title").origin.y;
    assert!(at(&mut vcx, "focus", b).is_none(), "not focused: no line");

    apply(&deck, cx, |l| l.focus(b));
    let mut vcx = window(cx, 0);
    assert!(at(&mut vcx, "focus", b).is_some(), "focused: the line");
    assert_eq!(at(&mut vcx, "tab", b).unwrap().origin.y, title, "focused: the title stays");
    assert_eq!(at(&mut vcx, "body", b).unwrap().origin.y, top, "focused: the body stays");
    assert_eq!(at(&mut vcx, "body", a).unwrap().origin.y, top, "side by side with a strip: bodies aligned");

    apply(&deck, cx, |l| l.move_tab(a, 1, Drop::Into(b)));
    let mut vcx = window(cx, 0);
    assert!(at(&mut vcx, "header", b).is_none() && at(&mut vcx, "tabs", b).is_some(), "two tabs: the strip");
    assert!(at(&mut vcx, "close", b).is_none(), "the ×s are the tabs'");
    assert_eq!(at(&mut vcx, "body", b).unwrap().origin.y, top, "a second tab: the body stays");

    apply(&deck, cx, |l| l.move_tab(b, 1, Drop::Into(a)));
    let mut vcx = window(cx, 0);
    assert!(at(&mut vcx, "header", b).is_some(), "back to one: the header");
    assert_eq!(at(&mut vcx, "body", b).unwrap().origin.y, top);
}

#[gpui::test]
fn the_headers_close_closes_the_tab_and_its_pane(cx: &mut TestAppContext) {
    let deck = setup(cx, devdeck_app::initial_layout);
    let c = pane_of(&deck, "demo/notes", cx);
    let mut vcx = window(cx, 0);
    let close = at(&mut vcx, "close", c).expect("×");
    vcx.simulate_click(close.center(), Modifiers::none());
    vcx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).layout().pane(c).is_err()), "the pane goes with its tab");
}

#[gpui::test]
fn the_headers_close_in_the_last_pane_leaves_it_empty(cx: &mut TestAppContext) {
    let deck = setup(cx, one_tab);
    let p = pane_of(&deck, "demo/notes", cx);
    let mut vcx = window(cx, 0);
    let close = at(&mut vcx, "close", p).expect("×");
    vcx.simulate_click(close.center(), Modifiers::none());
    vcx.run_until_parked();
    assert!(tabs(&deck, p, cx).is_empty(), "the last pane stays, empty");
    let mut vcx = window(cx, 0);
    assert!(at(&mut vcx, "header", p).is_some(), "an empty pane has the header");
    assert!(at(&mut vcx, "tab", p).is_none() && at(&mut vcx, "close", p).is_none(), "without a title or ×");
    assert!(vcx.debug_bounds("empty-message").is_some(), "its welcome");
}

#[gpui::test]
fn the_title_is_the_tab_by_mouse(cx: &mut TestAppContext) {
    let deck = setup(cx, devdeck_app::initial_layout);
    let b = pane_of(&deck, "matrix", cx);
    let c = pane_of(&deck, "demo/notes", cx);
    let w = cx.read(|cx| cx.windows()[0]);
    let mut vcx = window(cx, 0);
    let title = at(&mut vcx, "tab", b).expect("the title").center();
    let left = MouseButton::Left;
    vcx.simulate_event(gpui::MouseDownEvent { button: left, position: title, modifiers: Modifiers::none(), click_count: 2, first_mouse: false });
    vcx.simulate_event(gpui::MouseUpEvent { button: left, position: title, modifiers: Modifiers::none(), click_count: 2 });
    vcx.run_until_parked();
    assert_eq!(cx.read(|cx| deck.read(cx).layout().main().maximized), Some(b), "double-click maximizes");
    apply(&deck, cx, |l| l.toggle_maximize(b));

    // Right-click on the title, then on the rest of the header: the tab's menu, once.
    let mut vcx = window(cx, 0);
    right_click(&mut vcx, title);
    assert!(menu(&mut vcx, w).contains(&"Close others".to_string()), "the tab menu");
    vcx.simulate_keystrokes("escape");
    let header = at(&mut vcx, "header", b).unwrap();
    right_click(&mut vcx, point(header.origin.x + logical(4.), header.center().y));
    assert!(menu(&mut vcx, w).contains(&"Close others".to_string()), "the grip: the tab menu too");
    vcx.simulate_keystrokes("escape");

    // A strip's tab: its own menu, not replaced by the strip's pane menu.
    let a = pane_of(&deck, "demo/logs", cx);
    let tab = vcx.debug_bounds(Box::leak(format!("tab-{}-0", a.0).into_boxed_str())).expect("a tab").center();
    right_click(&mut vcx, tab);
    let items = menu(&mut vcx, w);
    assert!(items.contains(&"Close others".to_string()) && !items.contains(&"Split right".to_string()), "{items:?}");
    vcx.simulate_keystrokes("escape");

    let title = at(&mut vcx, "tab", c).unwrap().center();
    vcx.simulate_mouse_down(title, MouseButton::Middle, Modifiers::none());
    vcx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("demo/notes")).is_none()), "middle-click closes");
}

#[gpui::test]
fn a_tab_dropped_on_a_header_joins_its_pane(cx: &mut TestAppContext) {
    let deck = setup(cx, devdeck_app::initial_layout);
    let a = pane_of(&deck, "demo/logs", cx);
    let b = pane_of(&deck, "matrix", cx);
    let mut vcx = window(cx, 0);
    let from = vcx.debug_bounds(Box::leak(format!("tab-{}-1", a.0).into_boxed_str())).expect("a tab").center();
    let to = at(&mut vcx, "tab", b).unwrap().center();
    drag(&mut vcx, from, to);
    assert_eq!(tabs(&deck, b, cx), [ViewRef::new("matrix"), ViewRef::new("demo/long")], "after its tab");
    let mut vcx = window(cx, 0);
    assert!(at(&mut vcx, "header", b).is_none(), "now the strip");
}

#[gpui::test]
fn an_empty_panes_menus_are_the_panes(cx: &mut TestAppContext) {
    let deck = setup(cx, one_tab);
    let p = pane_of(&deck, "demo/notes", cx);
    apply(&deck, cx, |l| l.close_tab(p, 0).map(|_| ()));
    let w = cx.read(|cx| cx.windows()[0]);
    let mut vcx = window(cx, 0);
    let header = at(&mut vcx, "header", p).expect("the header");
    right_click(&mut vcx, header.center());
    assert!(menu(&mut vcx, w).iter().any(|l| l == "Split right"), "right-click: the pane menu");
    vcx.simulate_keystrokes("escape");
    vcx.update(|window, cx| window.dispatch_action(Box::new(devdeck_app::actions::ContextMenu), cx));
    vcx.run_until_parked();
    assert!(menu(&mut vcx, w).iter().any(|l| l == "Split right"), "the menu key: the pane menu");
    let open = vcx.update(|_, cx| devdeck_ui::overlay::layer(w, cx).read(cx).menu.clone()).unwrap();
    let idle: Vec<String> = vcx.read(|cx| open.read(cx).items().iter().filter(|i| i.disabled.as_deref() == Some("no tabs")).map(|i| i.label.to_string()).collect());
    assert_eq!(idle, ["Send pane to…", "Close pane"], "nothing to send or close");
}

#[gpui::test]
fn a_collapsed_one_tab_pane_is_its_header_and_a_click_expands_it(cx: &mut TestAppContext) {
    let deck = setup(cx, devdeck_app::initial_layout);
    let b = pane_of(&deck, "matrix", cx);
    apply(&deck, cx, |l| l.toggle_collapse(b));
    let mut vcx = window(cx, 0);
    assert!(at(&mut vcx, "header", b).is_some() && at(&mut vcx, "body", b).is_none(), "in a column split: the header only");
    let title = at(&mut vcx, "tab", b).unwrap().center();
    vcx.simulate_click(title, Modifiers::none());
    vcx.run_until_parked();
    assert!(cx.read(|cx| !deck.read(cx).layout().pane(b).unwrap().collapsed), "a click expands it");
}

#[gpui::test]
fn a_detached_one_tab_window_has_the_header_and_its_close_closes_the_window(cx: &mut TestAppContext) {
    let deck = setup(cx, devdeck_app::initial_layout);
    let c = pane_of(&deck, "demo/notes", cx);
    apply(&deck, cx, |l| l.detach(c, 0).map(|_| ()));
    assert_eq!(cx.windows().len(), 2);
    let p = pane_of(&deck, "demo/notes", cx);
    let mut vcx = window(cx, 1);
    assert!(at(&mut vcx, "header", p).is_some(), "the header");
    let close = at(&mut vcx, "close", p).expect("×");
    vcx.simulate_click(close.center(), Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the window goes with its tab");
}

#[gpui::test]
fn a_narrow_header_keeps_its_close(cx: &mut TestAppContext) {
    let deck = setup(cx, devdeck_app::initial_layout);
    let b = pane_of(&deck, "matrix", cx);
    let w = cx.read(|cx| cx.windows()[0]);
    cx.simulate_window_resize(w, size(logical(480.), logical(320.)));
    let mut vcx = window(cx, 0);
    let header = at(&mut vcx, "header", b).expect("the header");
    let close = at(&mut vcx, "close", b).expect("×");
    assert!(header.size.width < logical(200.), "narrow: {:?}", header.size.width);
    assert!(close.origin.x + close.size.width <= header.origin.x + header.size.width, "× inside the header: {close:?} in {header:?}");
    assert!(at(&mut vcx, "tab", b).unwrap().size.width > logical(24.), "the title still shows");
    assert!(at(&mut vcx, "buttons", b).unwrap().size.width < logical(1.), "the buttons gave way first");
}
