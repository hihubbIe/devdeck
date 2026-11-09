//! The test host itself: a component draws and takes clicks.

use crate::common;

use common::host;
use devdeck_ui::button;
use gpui::{InteractiveElement, IntoElement, Modifiers, StatefulInteractiveElement, TestAppContext, VisualTestContext};
use std::cell::Cell;
use std::rc::Rc;

#[gpui::test]
fn a_hosted_button_takes_a_click(cx: &mut TestAppContext) {
    let clicks = Rc::new(Cell::new(0));
    let c = clicks.clone();
    let window = host(cx, move |_, cx| {
        let c = c.clone();
        let t = devdeck_ui::theme(cx).clone();
        button("b", "press", &t).debug_selector(|| "press".into()).on_click(move |_, _, _| c.set(c.get() + 1)).into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    let b = vcx.debug_bounds("press").expect("drawn");
    vcx.simulate_click(b.center(), Modifiers::none());
    assert_eq!(clicks.get(), 1);
}
