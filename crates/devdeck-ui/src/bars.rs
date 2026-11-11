//! Small charts that sit in a row: a quantity against its bounds, a deployment's pods.
//! Sized from the theme (they follow the UI scale), colored by state only, and never
//! wider than they're given room for. `name` labels their parts for tests.

use crate::theme::Theme;
use gpui::{Hsla, InteractiveElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, div, prelude::*, relative};

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

/// One square per pod, ready / starting / failing / old (a rollout's); no count, the
/// squares say it (its tooltip spells it out: "2 of 3 ready").
pub fn pods_bar(name: &str, pods: &[PodMark], t: &Theme) -> gpui::Stateful<gpui::Div> {
    let wanted = pods.iter().filter(|p| **p != PodMark::Old).count();
    let ready = pods.iter().filter(|p| **p == PodMark::Ready).count();
    let mut row = div()
        .id(SharedString::from(format!("{name}-pods")))
        .debug_selector({
            let name = name.to_string();
            move || name
        })
        .flex_none()
        .flex()
        .items_center()
        .gap(t.hairline * 2.)
        .tooltip(crate::tip(format!("{ready} of {wanted} ready")));
    for (i, p) in pods.iter().enumerate() {
        let part = format!("{name}-{i}");
        row = row.child(pod_square(*p, t).debug_selector(move || part));
    }
    row
}

/// One pod's square, as a [`pods_bar`] draws it (a pod's row starts with it).
pub fn pod_square(mark: PodMark, t: &Theme) -> gpui::Div {
    let square = div().flex_none().size(t.size(9.)).rounded(t.hairline);
    match mark {
        PodMark::Ready => square.bg(t.ok),
        PodMark::Starting => square.border_1().border_dashed().border_color(t.warn),
        PodMark::Failing => square.bg(t.err),
        PodMark::Old => square.bg(t.dim),
    }
}
