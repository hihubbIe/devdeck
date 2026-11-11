//! Small charts that sit in a row: a quantity against its bounds, two counts either side
//! of a mark. Sized from the theme (they follow the UI scale), colored by state only, and
//! never wider than they're given room for. `name` labels their parts for tests.

use crate::theme::Theme;
use gpui::{Hsla, ParentElement, Styled, div, prelude::*, relative};

/// How many a [`split_bar`] side shows before it's full.
pub const SPLIT_FULL: u32 = 10;

/// A usage bar against a request (a tick) and a limit (the full width); without a limit
/// the bar spans the request, else what's used. Past its request it turns to a warning.
pub fn usage_bar(name: &str, used: u32, request: Option<u32>, limit: Option<u32>, t: &Theme) -> gpui::Div {
    let full = limit.or(request).unwrap_or(used).max(1) as f32;
    let fill = (used as f32 / full).min(1.0);
    let color: Hsla = if request.is_some_and(|r| used > r) { t.warn } else { t.ok };
    let (fill_name, tick_name) = (format!("{name}-fill"), format!("{name}-tick"));
    div()
        .debug_selector(|| name.to_string())
        .relative()
        .flex_none()
        .w(t.size(220.))
        .h(t.size(8.))
        .rounded(t.radius_s)
        .bg(t.line)
        .child(div().debug_selector(move || fill_name).absolute().left_0().top_0().h_full().w(relative(fill)).rounded(t.radius_s).bg(color))
        .when_some(request, |d, r| {
            d.child(div().debug_selector(move || tick_name).absolute().top_0().h_full().w(t.hairline).left(relative((r as f32 / full).min(1.0))).bg(t.fg))
        })
}

/// Two counts either side of a mark, each with its number: `behind` to the left (a
/// warning: something to take in), `ahead` to the right; a side is full at
/// [`SPLIT_FULL`]. A git branch against its upstream.
pub fn split_bar(name: &str, behind: u32, ahead: u32, t: &Theme) -> gpui::Div {
    let half = |n: u32| n.min(SPLIT_FULL) as f32 / SPLIT_FULL as f32;
    let side = |part: String, n: u32, color: Hsla, right: bool| {
        div()
            .flex_none()
            .w(t.size(48.))
            .h(t.size(8.))
            .flex()
            .when(!right, |d| d.justify_end())
            .bg(t.line)
            .child(div().debug_selector(move || part).h_full().w(relative(half(n))).bg(color))
    };
    let count = |text: String, n: u32, color: Hsla| div().flex_none().text_color(if n == 0 { t.dim } else { color }).child(text);
    div()
        .debug_selector(|| name.to_string())
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .child(count(format!("↓{behind}"), behind, t.warn))
        .child(side(format!("{name}-behind"), behind, t.warn, false))
        .child(div().flex_none().w(t.hairline).h(t.size(12.)).bg(t.mute))
        .child(side(format!("{name}-ahead"), ahead, t.ok, true))
        .child(count(format!("↑{ahead}"), ahead, t.ok))
}

/// One pod of a [`pods_bar`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PodMark {
    Ready,
    /// Not ready yet (starting, or waiting).
    Starting,
    Failing,
    /// Of the version a rollout replaces.
    Old,
}

/// One small square per pod, ready / starting / failing / old (a rollout's), then
/// `ready/wanted`.
pub fn pods_bar(name: &str, pods: &[PodMark], t: &Theme) -> gpui::Div {
    let wanted = pods.iter().filter(|p| **p != PodMark::Old).count();
    let ready = pods.iter().filter(|p| **p == PodMark::Ready).count();
    let side = t.size(9.);
    let mut row = div().debug_selector(|| name.to_string()).flex_none().flex().items_center().gap(t.hairline * 2.);
    for (i, p) in pods.iter().enumerate() {
        let part = format!("{name}-{i}");
        let square = div().debug_selector(move || part).flex_none().size(side).rounded(t.hairline);
        row = row.child(match p {
            PodMark::Ready => square.bg(t.ok),
            PodMark::Starting => square.border_1().border_dashed().border_color(t.warn),
            PodMark::Failing => square.bg(t.err),
            PodMark::Old => square.bg(t.dim),
        });
    }
    row.child(div().flex_none().ml_1().text_color(if ready == wanted { t.mute } else { t.warn }).child(format!("{ready}/{wanted}")))
}
