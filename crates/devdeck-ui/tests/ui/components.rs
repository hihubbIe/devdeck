//! The basic components on their own: buttons take clicks and keep mouse-down from the
//! title bar under them; icon buttons show their tooltip on hover; labels and section
//! headings stay on one line and truncate (R4.4); tree rows indent by level and their
//! chevron toggles without reaching the row.

use crate::common;

use common::{host, the_theme};
use devdeck_ui::{Icon, TREE_INDENT, button, icon_action_ready, icon_button, icon_toggle, label, logical, notify_button, section, tip, toggle_button, tree_row, under};
use gpui::{
    InteractiveElement, IntoElement, Modifiers, MouseButton, ParentElement, StatefulInteractiveElement, Styled, TestAppContext,
    VisualTestContext, div, point,
};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// Longer than GPUI's tooltip delay.
const HOVER: Duration = Duration::from_millis(600);

#[gpui::test]
fn a_button_clicks_and_keeps_mouse_down_from_what_is_under_it(cx: &mut TestAppContext) {
    let (clicks, parent_downs) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (c, p) = (clicks.clone(), parent_downs.clone());
    let window = host(cx, move |_, cx| {
        let (c, p) = (c.clone(), p.clone());
        let t = devdeck_ui::theme(cx).clone();
        // Like the title bar: mouse down on it would start a window move.
        div()
            .p(logical(20.))
            .on_mouse_down(MouseButton::Left, move |_, _, _| p.set(p.get() + 1))
            .child(button("b", "press", &t).debug_selector(|| "b".into()).on_click(move |_, _, _| c.set(c.get() + 1)))
            .into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    let b = vcx.debug_bounds("b").expect("drawn");
    vcx.simulate_click(b.center(), Modifiers::none());
    assert_eq!((clicks.get(), parent_downs.get()), (1, 0));
}

#[gpui::test]
fn a_toggle_button_clicks_whatever_its_state(cx: &mut TestAppContext) {
    let on = Rc::new(Cell::new(false));
    let o = on.clone();
    let window = host(cx, move |_, cx| {
        let o = o.clone();
        let t = devdeck_ui::theme(cx).clone();
        toggle_button("t", "follow", o.get(), &t).debug_selector(|| "t".into()).on_click(move |_, _, _| o.set(!o.get())).into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    for want in [true, false, true] {
        let b = vcx.debug_bounds("t").expect("drawn");
        vcx.simulate_click(b.center(), Modifiers::none());
        assert_eq!(on.get(), want);
    }
}

#[gpui::test]
fn an_icon_toggle_clicks_whatever_its_state(cx: &mut TestAppContext) {
    let on = Rc::new(Cell::new(false));
    let o = on.clone();
    let window = host(cx, move |_, cx| {
        let o = o.clone();
        let t = devdeck_ui::theme(cx).clone();
        div().p(logical(20.)).child(icon_toggle("w", Icon::Wrap, "wrap long lines", o.get(), &t).debug_selector(|| "w".into()).on_click(move |_, _, _| o.set(!o.get()))).into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    for want in [true, false] {
        let b = vcx.debug_bounds("w").expect("drawn");
        vcx.simulate_click(b.center(), Modifiers::none());
        assert_eq!(on.get(), want);
    }
}

#[gpui::test]
fn an_icon_button_shows_its_tooltip_on_hover(cx: &mut TestAppContext) {
    let window = host(cx, |_, cx| {
        let t = devdeck_ui::theme(cx).clone();
        div().p(logical(40.)).child(icon_button("i", devdeck_ui::Icon::Close, "close", &t).debug_selector(|| "i".into())).into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    let b = vcx.debug_bounds("i").expect("drawn");
    vcx.simulate_mouse_move(b.center(), None, Modifiers::none());
    assert!(vcx.debug_bounds("tooltip").is_none(), "not at once");
    vcx.executor().advance_clock(HOVER);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("tooltip").is_some(), "after the delay");
    // Moving away hides it.
    vcx.simulate_mouse_move(point(b.right() + logical(200.), b.bottom() + logical(200.)), None, Modifiers::none());
    vcx.executor().advance_clock(HOVER);
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("tooltip").is_none());
}

#[gpui::test]
fn a_label_stays_on_one_line_and_its_tooltip_has_the_full_text(cx: &mut TestAppContext) {
    let long = "a label much longer than the narrow box it is drawn in, which must truncate";
    let window = host(cx, move |_, _| {
        div()
            .w(logical(120.))
            .child(div().id("holder").debug_selector(|| "label".into()).tooltip(tip(long)).child(label(long)))
            .into_any_element()
    });
    let t = the_theme(cx);
    let mut vcx = VisualTestContext::from_window(window, cx);
    let b = vcx.debug_bounds("label").expect("drawn");
    assert!(b.size.height < t.line_height * 1.5, "one line, never wrapped: {b:?}");
    assert!(b.size.width <= logical(120.), "within its box: {b:?}");
    vcx.simulate_mouse_move(b.center(), None, Modifiers::none());
    vcx.executor().advance_clock(HOVER);
    vcx.run_until_parked();
    let tooltip = vcx.debug_bounds("tooltip").expect("full text on hover");
    assert!(tooltip.size.width > logical(120.), "the tooltip isn't truncated: {tooltip:?}");
}

#[gpui::test]
fn a_section_heading_is_one_line_across_its_container(cx: &mut TestAppContext) {
    let window = host(cx, |_, cx| {
        let t = devdeck_ui::theme(cx).clone();
        div()
            .w(logical(150.))
            .child(div().debug_selector(|| "section".into()).child(section("a heading too long for one hundred and fifty pixels", &t)))
            .into_any_element()
    });
    let t = the_theme(cx);
    let mut vcx = VisualTestContext::from_window(window, cx);
    let b = vcx.debug_bounds("section").expect("drawn");
    assert_eq!(b.size.width, logical(150.));
    assert!(b.size.height < t.line_height * 2., "one line plus its spacing: {b:?}");
}

#[gpui::test]
fn tree_rows_indent_by_level_and_line_up_with_or_without_children(cx: &mut TestAppContext) {
    let window = host(cx, |_, cx| {
        let t = devdeck_ui::theme(cx).clone();
        div()
            .w(logical(200.))
            .child(tree_row("top", 0, Some(false), |_, _| {}, &t).child(div().debug_selector(|| "top-label".into()).child("top")))
            .child(tree_row("leaf", 0, None, |_, _| {}, &t).child(div().debug_selector(|| "leaf-label".into()).child("leaf")))
            .child(tree_row("deep", 2, Some(true), |_, _| {}, &t).child(div().debug_selector(|| "deep-label".into()).child("deep")))
            .into_any_element()
    });
    let t = the_theme(cx);
    let mut vcx = VisualTestContext::from_window(window, cx);
    let x = |vcx: &mut VisualTestContext, s: &'static str| vcx.debug_bounds(s).expect(s).left();
    assert_eq!(x(&mut vcx, "top-label"), x(&mut vcx, "leaf-label"), "a leaf's label lines up with a folder's");
    assert_eq!(x(&mut vcx, "deep-chevron") - x(&mut vcx, "top-chevron"), t.size(TREE_INDENT) * 2., "two levels in");
    for s in ["top", "leaf", "deep"] {
        assert_eq!(vcx.debug_bounds(s).expect(s).size.height, t.row_h, "{s}: one row high");
    }
}

#[gpui::test]
fn a_tree_chevron_toggles_without_reaching_the_row(cx: &mut TestAppContext) {
    let (open, row_downs) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(0)));
    let (o, r) = (open.clone(), row_downs.clone());
    let window = host(cx, move |_, cx| {
        let (o, r) = (o.clone(), r.clone());
        let t = devdeck_ui::theme(cx).clone();
        let toggle = o.clone();
        div()
            .w(logical(200.))
            .child(
                tree_row("folder", 1, Some(o.get()), move |_, _| toggle.set(!toggle.get()), &t)
                    .on_mouse_down(MouseButton::Left, move |_, _, _| r.set(r.get() + 1))
                    .child(div().debug_selector(|| "folder-label".into()).child("folder")),
            )
            .child(tree_row("leaf", 1, None, |_, _| panic!("a leaf has no chevron"), &t).child("leaf"))
            .into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    for want in [true, false] {
        let c = vcx.debug_bounds("folder-chevron").expect("drawn");
        vcx.simulate_click(c.center(), Modifiers::none());
        assert_eq!((open.get(), row_downs.get()), (want, 0), "the chevron toggles, the row doesn't see it");
    }
    let label = vcx.debug_bounds("folder-label").expect("drawn");
    vcx.simulate_click(label.center(), Modifiers::none());
    assert_eq!((open.get(), row_downs.get()), (false, 1), "the rest of the row is the row's");
    let blank = vcx.debug_bounds("leaf-chevron").expect("drawn");
    vcx.simulate_click(blank.center(), Modifiers::none());
}

/// The rocket and the notify button: both take a click whether ready or not (a dim action
/// says why on its click), keep their size in either state (R9.10), and the notify button
/// knows where it was drawn, so a key opens its menu under it.
#[gpui::test]
fn ready_actions_and_notify_buttons_click_in_any_state_and_keep_their_size(cx: &mut TestAppContext) {
    let clicks = Rc::new(Cell::new(0));
    let state = Rc::new(Cell::new(false));
    let at: Rc<Cell<Option<gpui::Bounds<gpui::Pixels>>>> = Rc::default();
    let (c, st, a) = (clicks.clone(), state.clone(), at.clone());
    let window = host(cx, move |_, cx| {
        let t = devdeck_ui::theme(cx).clone();
        let (c1, c2) = (c.clone(), c.clone());
        div()
            .p(logical(20.))
            .flex()
            .child(icon_action_ready("deploy", Icon::Deploy, "the tag is unchanged", st.get(), &t).debug_selector(|| "deploy".into()).on_click(move |_, _, _| c1.set(c1.get() + 1)))
            .child(notify_button("notify", st.get(), a.clone(), &t).debug_selector(|| "notify".into()).on_click(move |_, _, _| c2.set(c2.get() + 1)))
            .into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    let (deploy, notify) = (vcx.debug_bounds("deploy").expect("drawn"), vcx.debug_bounds("notify").expect("drawn"));
    assert_eq!(at.get(), Some(notify), "it recorded where it was drawn");
    assert_eq!(under(at.get()), notify.bottom_left(), "a menu opens under it");
    vcx.simulate_click(deploy.center(), Modifiers::none());
    vcx.simulate_click(notify.center(), Modifiers::none());
    assert_eq!(clicks.get(), 2, "dim, crossed bell: still clickable");
    state.set(true);
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    assert_eq!((vcx.debug_bounds("deploy"), vcx.debug_bounds("notify")), (Some(deploy), Some(notify)), "ready, ticked: nothing moved or resized");
}
