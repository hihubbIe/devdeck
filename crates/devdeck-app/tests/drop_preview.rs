//! Dropping beside a whole column (R3.13): the drop layer under the panes, its preview
//! (the exact rectangle the dragged tab lands in), and the keyboard way (R5.1).

use devdeck_app::deck::Deck;
use devdeck_app::shell::Shell;
use devdeck_core::layout::{Drop, Edge, Layout, Node, PaneId, ViewRef};
use devdeck_ui::logical;
use gpui::{Bounds, Entity, Modifiers, MouseButton, Pixels, Point, TestAppContext, VisualTestContext, point};

/// Two stacked panes: `notes` on top, `long` and `logs` below.
fn stacked() -> Layout {
    let mut l = Layout::new();
    let a = l.main().focused;
    for v in ["demo/notes", "demo/long", "demo/logs"] {
        l.open(ViewRef::new(v), a).unwrap();
    }
    l.move_tab(a, 1, Drop::Edge(a, Edge::Bottom)).unwrap();
    let (b, _) = l.find_view(&ViewRef::new("demo/long")).unwrap();
    l.move_tab(a, 1, Drop::Into(b)).unwrap();
    l
}

fn setup(cx: &mut TestAppContext) -> Entity<Deck> {
    let dir = cx.update(|cx| devdeck_app::test_dir("drop-preview", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&dir, cx));
    let shell = cx.update(|cx| Shell::start_single(stacked, cx));
    cx.run_until_parked();
    cx.read(|cx| shell.read(cx).active_deck())
}

fn bounds(vcx: &mut VisualTestContext, name: String) -> Option<Bounds<Pixels>> {
    vcx.debug_bounds(Box::leak(name.into_boxed_str()))
}

fn pane_of(deck: &Entity<Deck>, view: &str, vcx: &mut VisualTestContext) -> PaneId {
    vcx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new(view)).unwrap().0)
}

/// From the header (or strip) top to the body's bottom.
fn pane_bounds(vcx: &mut VisualTestContext, p: PaneId) -> Bounds<Pixels> {
    let head = bounds(vcx, format!("pane-header-{}", p.0)).expect("a one-tab pane has a header");
    let body = bounds(vcx, format!("pane-body-{}", p.0)).expect("its body");
    Bounds { origin: head.origin, size: gpui::size(head.size.width, body.origin.y + body.size.height - head.origin.y) }
}

fn near(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
    let close = |x: Pixels, y: Pixels| (x - y).abs() <= logical(1.);
    close(a.origin.x, b.origin.x) && close(a.origin.y, b.origin.y) && close(a.size.width, b.size.width) && close(a.size.height, b.size.height)
}

#[gpui::test]
fn a_tab_dropped_on_the_windows_right_strip_gets_a_full_height_column_where_the_preview_said(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let window = cx.read(|cx| cx.windows()[0]);
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    let b = pane_of(&deck, "demo/logs", &mut vcx);
    let i = vcx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("demo/logs")).unwrap().1);
    let tab = bounds(&mut vcx, format!("tab-{}-{i}", b.0)).expect("the tab").center();
    let panes = bounds(&mut vcx, "panes".into()).expect("the panes");
    let body = bounds(&mut vcx, format!("pane-body-{}", b.0)).expect("b's body");
    // 2 px from the window's right edge, in b's body: the outer strip.
    let to: Point<Pixels> = point(panes.origin.x + panes.size.width - logical(2.), body.center().y);
    vcx.simulate_mouse_down(tab, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(tab + point(logical(20.), logical(0.)), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    let preview = bounds(&mut vcx, "drop-preview".into()).expect("the preview while dragging");
    assert!((preview.size.height - panes.size.height).abs() <= logical(1.), "full height: {preview:?} in {panes:?}");
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    assert!(bounds(&mut vcx, "drop-preview".into()).is_none(), "gone with the drag");
    let logs = pane_of(&deck, "demo/logs", &mut vcx);
    let landed = pane_bounds(&mut vcx, logs);
    assert!(near(preview, landed), "landed where the preview said: {preview:?} vs {landed:?}");
    // The stack stays on the left, the new column on the right.
    let root = vcx.read(|cx| deck.read(cx).layout().main().root.clone());
    let Node::Split(s) = root else { panic!("a split") };
    assert_eq!((s.axis, s.children.len()), (devdeck_core::layout::Axis::Row, 2), "{s:?}");
}

#[gpui::test]
fn move_tab_to_window_edge_does_it_by_keyboard(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let window = cx.read(|cx| cx.windows()[0]);
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    let b = pane_of(&deck, "demo/logs", &mut vcx);
    let i = vcx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("demo/logs")).unwrap().1);
    vcx.update(|_, cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.activate(b, i))));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.dispatch_action(Box::new(devdeck_app::actions::MoveTabToRightEdge), cx));
    vcx.run_until_parked();
    let root = vcx.read(|cx| deck.read(cx).layout().main().root.clone());
    let Node::Split(s) = root else { panic!("a split") };
    assert_eq!(s.axis, devdeck_core::layout::Axis::Row);
    assert!(matches!(s.children[1].node, Node::Pane(p) if vcx.read(|cx| deck.read(cx).layout().pane(p).unwrap().tabs == [ViewRef::new("demo/logs")])));
}

fn drag(vcx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>) {
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(from + point(logical(20.), logical(0.)), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
}

#[gpui::test]
fn a_pane_dragged_by_its_grip_and_a_tab_dropped_in_a_bodys_centre_go_through_the_layer(cx: &mut TestAppContext) {
    let deck = setup(cx);
    let window = cx.read(|cx| cx.windows()[0]);
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    // The top pane (notes, one tab) by its grip, onto the window's right strip.
    let a = pane_of(&deck, "demo/notes", &mut vcx);
    let head = bounds(&mut vcx, format!("pane-header-{}", a.0)).expect("its header");
    let grip = point(head.origin.x + logical(8.), head.center().y);
    let panes = bounds(&mut vcx, "panes".into()).unwrap();
    let b = pane_of(&deck, "demo/logs", &mut vcx);
    let body = bounds(&mut vcx, format!("pane-body-{}", b.0)).unwrap();
    drag(&mut vcx, grip, point(panes.origin.x + panes.size.width - logical(2.), body.center().y));
    assert_eq!(pane_of(&deck, "demo/notes", &mut vcx), a, "the same pane moved");
    let notes = pane_bounds(&mut vcx, a);
    assert!((notes.size.height - panes.size.height).abs() <= logical(1.), "full height on the right: {notes:?}");
    // A tab of the left pane dropped in the centre of notes' body: it joins notes.
    let i = vcx.read(|cx| deck.read(cx).layout().find_view(&ViewRef::new("demo/long")).unwrap().1);
    let tab = bounds(&mut vcx, format!("tab-{}-{i}", b.0)).expect("a tab").center();
    let centre = bounds(&mut vcx, format!("pane-body-{}", a.0)).unwrap().center();
    drag(&mut vcx, tab, centre);
    assert_eq!(pane_of(&deck, "demo/long", &mut vcx), a, "into notes' pane");
}
