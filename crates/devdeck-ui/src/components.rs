//! Shared components (R9.3). Views compose these instead of styling raw elements, so
//! the look stays identical everywhere and follows the theme and UI scale.

use crate::theme::{Theme, theme};
use gpui::{
    AnyView, App, AppContext, Context, ElementId, IntoElement, MouseButton, ParentElement, Render, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Window, div, prelude::*,
};

/// Tooltip with the full text of a truncated label (R4.4).
pub struct Tip(pub SharedString);

impl Render for Tip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .debug_selector(|| "tooltip".into())
            .px_2()
            .py_1()
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .rounded(t.radius_m)
            .text_color(t.fg)
            .text_size(t.text)
            .whitespace_nowrap()
            .child(self.0.clone())
    }
}

/// Builder for `.tooltip(...)`.
pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

/// Floating label shown while dragging a tab or a pane.
pub struct DragGhost(pub SharedString);

impl Render for DragGhost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div()
            .px_3()
            .h(t.toolbar_h)
            .flex()
            .items_center()
            .bg(t.accent_bg)
            .border_1()
            .border_color(t.accent)
            .rounded(t.radius_m)
            .text_color(t.hi)
            .text_size(t.text)
            .font_family(t.font.clone())
            .whitespace_nowrap()
            .child(self.0.clone())
    }
}

/// Nothing to show (divider drags).
pub struct Empty;

impl Render for Empty {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// Single-line text that truncates with an ellipsis and never wraps (R4.4).
pub fn label(text: impl Into<SharedString>) -> gpui::Div {
    div().min_w_0().truncate().child(text.into())
}

/// Text button for title bars and toolbars. Mouse-down doesn't reach the parent, so a
/// button inside a draggable title bar doesn't start a window move.
pub fn button(id: impl Into<ElementId>, text: impl Into<SharedString>, t: &Theme) -> Stateful<gpui::Div> {
    let (hover, fg) = (t.hover, t.fg);
    div()
        .id(id)
        .px_2()
        .h(t.button_h)
        .flex()
        .flex_none()
        .items_center()
        .rounded(t.radius_m)
        .text_color(t.mute)
        .cursor_pointer()
        .whitespace_nowrap()
        .hover(move |s| s.bg(hover).text_color(fg))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(text.into())
}

/// A button showing it is the selected option of a group.
pub fn toggle_button(id: impl Into<ElementId>, text: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<gpui::Div> {
    let b = button(id, text, t).border_1();
    if on {
        b.border_color(t.accent).bg(t.accent_bg).text_color(t.hi)
    } else {
        b.border_color(t.line)
    }
}

/// Square icon-sized button with a tooltip.
pub fn icon_button(id: impl Into<ElementId>, glyph: &'static str, tooltip: &'static str, t: &Theme) -> Stateful<gpui::Div> {
    let (hover, fg) = (t.hover, t.fg);
    div()
        .id(id)
        .w(t.icon_button)
        .h(t.icon_button)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(t.radius_s)
        .text_color(t.mute)
        .cursor_pointer()
        .hover(move |s| s.bg(hover).text_color(fg))
        .tooltip(tip(tooltip))
        .child(glyph)
}

/// How far each level of a tree is indented (logical pixels, before the UI scale).
pub const TREE_INDENT: f32 = 12.;

/// One row of a tree: indented by `depth`, then a chevron cell, ▸ closed or ▾ open when
/// the row has children (`open: Some(..)`), blank otherwise so labels line up. A mouse
/// down on the chevron toggles (`on_toggle`) and doesn't reach the row, so the row's own
/// handlers (select, open, drag) stay the caller's, as does its content. `name` is the
/// row's element id and test selector; the chevron's is `{name}-chevron`.
pub fn tree_row(
    name: impl Into<SharedString>,
    depth: usize,
    open: Option<bool>,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
    t: &Theme,
) -> Stateful<gpui::Div> {
    let name: SharedString = name.into();
    let (row_selector, chevron_selector) = (name.to_string(), format!("{name}-chevron"));
    let chevron = div()
        .debug_selector(move || chevron_selector.clone())
        .flex_none()
        .w(t.size(TREE_INDENT))
        .flex()
        .justify_center()
        .text_color(t.dim);
    let chevron = match open {
        Some(open) => chevron
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                on_toggle(window, cx);
            })
            .child(if open { "▾" } else { "▸" }),
        None => chevron,
    };
    div()
        .id(ElementId::Name(name))
        .debug_selector(move || row_selector.clone())
        .h(t.row_h)
        .w_full()
        .pl(t.size(8.) + t.size(TREE_INDENT) * depth as f32)
        .pr_3()
        .flex()
        .items_center()
        .gap_1()
        .whitespace_nowrap()
        .child(chevron)
}

/// Section heading inside a view: muted text over a soft hairline.
pub fn section(text: impl Into<SharedString>, t: &Theme) -> gpui::Div {
    div()
        .w_full()
        .mt_3()
        .mb_1()
        .pb_1()
        .border_b_1()
        .border_color(t.line_soft)
        .text_color(t.mute)
        .child(label(text))
}
