//! The basic components on their own: buttons take clicks and keep mouse-down from the
//! title bar under them; icon buttons show their tooltip on hover; labels and section
//! headings stay on one line and truncate (R4.4).

mod common;

use common::{host, the_theme};
use devdeck_ui::{button, icon_button, label, logical, section, tip, toggle_button};
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
fn an_icon_button_shows_its_tooltip_on_hover(cx: &mut TestAppContext) {
    let window = host(cx, |_, cx| {
        let t = devdeck_ui::theme(cx).clone();
        div().p(logical(40.)).child(icon_button("i", "×", "close", &t).debug_selector(|| "i".into())).into_any_element()
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
