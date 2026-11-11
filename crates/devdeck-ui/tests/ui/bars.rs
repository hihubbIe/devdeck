//! The small charts: a usage bar fills to what's used against its limit, its tick at the
//! request; a split bar fills each side to its count, full at `SPLIT_FULL`; neither grows
//! past its width.

use crate::common;

use common::host;
use devdeck_ui::{SPLIT_FULL, split_bar, usage_bar};
use gpui::{IntoElement, TestAppContext, VisualTestContext};

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.5
}

#[gpui::test]
fn a_usage_bar_fills_to_its_limit_with_a_tick_at_the_request(cx: &mut TestAppContext) {
    let window = host(cx, |_, cx| usage_bar("u", 50, Some(25), Some(100), devdeck_ui::theme(cx)).into_any_element());
    let mut vcx = VisualTestContext::from_window(window, cx);
    let bar = vcx.debug_bounds("u").expect("drawn");
    let fill = vcx.debug_bounds("u-fill").expect("drawn");
    let tick = vcx.debug_bounds("u-tick").expect("drawn");
    let w = f32::from(bar.size.width);
    assert!(near(f32::from(fill.size.width), w * 0.5), "half its limit");
    assert!(near(f32::from(tick.origin.x - bar.origin.x), w * 0.25), "the request a quarter in");
}

#[gpui::test]
fn a_usage_bar_past_its_limit_stays_its_width(cx: &mut TestAppContext) {
    let window = host(cx, |_, cx| usage_bar("u", 300, None, Some(100), devdeck_ui::theme(cx)).into_any_element());
    let mut vcx = VisualTestContext::from_window(window, cx);
    let bar = vcx.debug_bounds("u").expect("drawn");
    let fill = vcx.debug_bounds("u-fill").expect("drawn");
    assert!(near(f32::from(fill.size.width), f32::from(bar.size.width)));
    assert!(vcx.debug_bounds("u-tick").is_none(), "no request, no tick");
}

#[gpui::test]
fn a_split_bar_fills_each_side_to_its_count(cx: &mut TestAppContext) {
    let window = host(cx, |_, cx| split_bar("s", 0, SPLIT_FULL / 2, devdeck_ui::theme(cx)).into_any_element());
    let mut vcx = VisualTestContext::from_window(window, cx);
    let behind = vcx.debug_bounds("s-behind").expect("drawn");
    let ahead = vcx.debug_bounds("s-ahead").expect("drawn");
    assert!(near(f32::from(behind.size.width), 0.), "nothing behind");
    assert!(f32::from(ahead.size.width) > 0., "half ahead");
    assert!(behind.origin.x < ahead.origin.x, "behind on the left");
}

#[gpui::test]
fn a_split_bar_side_is_full_at_its_cap_and_no_wider(cx: &mut TestAppContext) {
    let window = host(cx, |_, cx| split_bar("s", SPLIT_FULL, SPLIT_FULL * 3, devdeck_ui::theme(cx)).into_any_element());
    let mut vcx = VisualTestContext::from_window(window, cx);
    let behind = vcx.debug_bounds("s-behind").expect("drawn");
    let ahead = vcx.debug_bounds("s-ahead").expect("drawn");
    assert!(near(f32::from(behind.size.width), f32::from(ahead.size.width)), "both full, the same width");
}
