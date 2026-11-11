//! The small charts: a usage bar fills to what's used against its limit, its tick at the
//! request, never past its width; a pods bar draws a square per pod.

use crate::common;

use common::host;
use devdeck_ui::usage_bar;
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
fn a_pods_bar_draws_one_square_per_pod_in_order(cx: &mut TestAppContext) {
    use devdeck_ui::{PodMark, pods_bar};
    let pods = [PodMark::Ready, PodMark::Starting, PodMark::Failing, PodMark::Old];
    let window = host(cx, move |_, cx| pods_bar("p", &pods, devdeck_ui::theme(cx)).into_any_element());
    let mut vcx = VisualTestContext::from_window(window, cx);
    let squares: Vec<_> = ["p-0", "p-1", "p-2", "p-3"].into_iter().map(|n| vcx.debug_bounds(n).expect("drawn")).collect();
    assert!(squares.windows(2).all(|w| w[0].origin.x < w[1].origin.x), "in order, left to right");
    assert!(squares.iter().all(|s| s.size == squares[0].size), "the same size whatever their state (R9.10)");
    assert!(vcx.debug_bounds("p-4").is_none(), "one per pod");
}
