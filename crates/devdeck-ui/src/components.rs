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

/// A key hint like `ctrl k`.
pub fn kbd(keys: impl Into<SharedString>, t: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .px_1()
        .border_1()
        .border_color(t.line)
        .rounded(t.radius_s)
        .text_size(t.text_small)
        .text_color(t.mute)
        .whitespace_nowrap()
        .child(keys.into())
}
