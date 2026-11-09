//! Scroll containers (R9.6): a trackpad gesture moves the axis it started on, even when
//! some of its events lean the other way; the plain wheel scrolls down, and leaves a
//! sideways-only area alone (shift+wheel is the platform's).
//!
//! Events are `TouchPhase::Moved`, as Linux sends them: a gesture is the run of events
//! closer than GPUI's separation (28 ms), so each test sends its gesture at once.

use crate::common;

use common::host;
use devdeck_ui::{Scrolls, logical, scroll_list};
use gpui::{
    AnyWindowHandle, InteractiveElement, IntoElement, ListHorizontalSizingBehavior, Modifiers, ParentElement, ScrollDelta, ScrollEvent,
    ScrollHandle, StatefulInteractiveElement, Styled, TestAppContext, TouchPhase, UniformListScrollHandle, VisualTestContext, div, point,
};

/// A 200×100 area over 1000×1000 of content, scrolling both ways.
fn both_ways(cx: &mut TestAppContext) -> (AnyWindowHandle, ScrollHandle) {
    let handle = ScrollHandle::new();
    let h = handle.clone();
    let window = host(cx, move |_, _| {
        div()
            .id("both")
            .w(logical(200.))
            .h(logical(100.))
            .scroll_xy()
            .track_scroll(&h)
            .child(div().w(logical(1000.)).h(logical(1000.)))
            .into_any_element()
    });
    (window, handle)
}

/// Sends `deltas` as one gesture, over the area's middle.
fn gesture(vcx: &mut VisualTestContext, deltas: &[ScrollDelta]) {
    let position = point(logical(100.), logical(50.));
    for &delta in deltas {
        vcx.simulate_event(ScrollEvent { position, delta, modifiers: Modifiers::none(), touch_phase: TouchPhase::Moved, source: Default::default() });
    }
    vcx.run_until_parked();
}

fn swipe(x: f32, y: f32) -> ScrollDelta {
    ScrollDelta::Pixels(point(logical(x), logical(y)))
}

/// Sideways, with a wobble: the second event leans vertical.
fn sideways() -> [ScrollDelta; 3] {
    [swipe(-10., -1.), swipe(-2., -3.), swipe(-8., -2.)]
}

#[gpui::test]
fn a_sideways_swipe_never_moves_up_or_down(cx: &mut TestAppContext) {
    let (window, handle) = both_ways(cx);
    let mut vcx = VisualTestContext::from_window(window, cx);
    gesture(&mut vcx, &sideways());
    let offset = handle.offset();
    assert_eq!(offset.y, logical(0.), "{offset:?}");
    assert_eq!(offset.x, logical(-20.), "{offset:?}");
}

#[gpui::test]
fn a_wide_list_keeps_a_sideways_swipe_sideways(cx: &mut TestAppContext) {
    let handle = UniformListScrollHandle::new();
    let h = handle.clone();
    let window = host(cx, move |_, _| {
        div()
            .w(logical(200.))
            .h(logical(100.))
            .flex()
            .child(
                scroll_list("wide", 100, |range, _, _| range.map(|_| div().h(logical(20.)).w(logical(1000.))).collect())
                    .track_scroll(&h)
                    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                    .with_width_from_item(Some(0))
                    .scroll_xy()
                    .size_full(),
            )
            .into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    gesture(&mut vcx, &sideways());
    let offset = handle.0.borrow().base_handle.offset();
    assert_eq!(offset.y, logical(0.), "{offset:?}");
    assert!(offset.x < logical(0.), "{offset:?}");
}

#[gpui::test]
fn the_wheel_scrolls_down(cx: &mut TestAppContext) {
    let (window, handle) = both_ways(cx);
    let mut vcx = VisualTestContext::from_window(window, cx);
    gesture(&mut vcx, &[ScrollDelta::Lines(point(0., -1.))]);
    let offset = handle.offset();
    assert!(offset.y < logical(0.) && offset.x == logical(0.), "{offset:?}");
}

#[gpui::test]
fn the_wheel_leaves_a_sideways_only_area_alone(cx: &mut TestAppContext) {
    let handle = ScrollHandle::new();
    let h = handle.clone();
    let window = host(cx, move |_, _| {
        div()
            .id("sideways")
            .w(logical(200.))
            .h(logical(100.))
            .scroll_x()
            .track_scroll(&h)
            .child(div().w(logical(1000.)).h(logical(100.)))
            .into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    gesture(&mut vcx, &[ScrollDelta::Lines(point(0., -1.))]);
    assert_eq!(handle.offset().x, logical(0.), "the plain wheel is up and down only");
    gesture(&mut vcx, &[ScrollDelta::Lines(point(-1., 0.))]);
    assert!(handle.offset().x < logical(0.), "a sideways wheel (shift+wheel) scrolls it");
}
