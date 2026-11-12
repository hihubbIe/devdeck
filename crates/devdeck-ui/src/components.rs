//! Shared components (R9.3). Views compose these instead of styling raw elements, so
//! the look stays identical everywhere and follows the theme and UI scale.

use crate::icons::{Icon, icon};
use crate::theme::{Theme, theme};
use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, Context, ElementId, FocusHandle, Hsla, IntoElement, MouseButton, ParentElement, Pixels, Point, Render, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Svg, Window, div, prelude::*,
};
use std::cell::Cell;
use std::rc::Rc;

/// The hover group of [`button`] and [`icon_button`]: their icons brighten with them.
const BUTTON: &str = "button";

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
/// button inside a draggable title bar doesn't start a window move. Icons go in with
/// [`button_icon`].
pub fn button(id: impl Into<ElementId>, text: impl Into<SharedString>, t: &Theme) -> Stateful<gpui::Div> {
    button_frame(id, t).child(text.into())
}

/// A [`button`] whose icon comes before its text.
pub fn labeled_button(id: impl Into<ElementId>, i: Icon, text: impl Into<SharedString>, t: &Theme) -> Stateful<gpui::Div> {
    button_frame(id, t).child(button_icon(i, t)).child(text.into())
}

fn button_frame(id: impl Into<ElementId>, t: &Theme) -> Stateful<gpui::Div> {
    let (hover, fg) = (t.hover, t.fg);
    div()
        .id(id)
        .group(BUTTON)
        .px_2()
        .h(t.button_h)
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .rounded(t.radius_m)
        .text_color(t.mute)
        .cursor_pointer()
        .whitespace_nowrap()
        .hover(move |s| s.bg(hover).text_color(fg))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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

/// A control the keys reach (a button, a toggle): `h` tracked, a border always, which
/// the focus only colors (R9.10); `bordered` when the control draws one already.
pub fn focus_ring(el: Stateful<gpui::Div>, h: &FocusHandle, bordered: bool, t: &Theme) -> Stateful<gpui::Div> {
    let ring = t.focus;
    let el = if bordered { el } else { el.border_1().border_color(gpui::transparent_black()) };
    el.track_focus(h).focus(move |s| s.border_color(ring))
}

/// A [`toggle_button`] whose label keeps `color`, on, off and hovered: the color says
/// something (a notification option's severity).
pub fn toggle_button_colored(id: impl Into<ElementId>, text: impl Into<SharedString>, on: bool, color: Hsla, t: &Theme) -> Stateful<gpui::Div> {
    let hover = t.hover;
    div()
        .id(id)
        .group(BUTTON)
        .px_2()
        .h(t.button_h)
        .flex()
        .flex_none()
        .items_center()
        .rounded(t.radius_m)
        .border_1()
        .border_color(if on { t.accent } else { t.line })
        .when(on, |d| d.bg(t.accent_bg))
        .text_color(color)
        .cursor_pointer()
        .whitespace_nowrap()
        .hover(move |s| s.bg(hover))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(text.into())
}

/// An icon in a [`button`] (before or after its text, or alone): muted, bright with
/// the button under the pointer.
pub fn button_icon(i: Icon, t: &Theme) -> Svg {
    let fg = t.fg;
    icon(i, t.mute, t).group_hover(BUTTON, move |s| s.text_color(fg))
}

/// [`icon_button`] for an action that may be refused now: its tooltip names it, or says
/// why it can't run (R5.5), and its icon is dimmed then (still clickable: a click says
/// why).
pub fn icon_action(id: impl Into<ElementId>, i: Icon, tooltip: impl Into<SharedString>, enabled: bool, t: &Theme) -> Stateful<gpui::Div> {
    let child = if enabled { button_icon(i, t) } else { icon(i, t.dim, t) };
    icon_frame(id, t).tooltip(tip(tooltip)).child(child)
}

/// "Go there": where it leads (an activity's icon, a pulse's) with a small arrow in its
/// corner. Pivots from one view to another, and "open in pulse".
pub fn go_action(id: impl Into<ElementId>, i: Icon, tooltip: impl Into<SharedString>, t: &Theme) -> Stateful<gpui::Div> {
    let corner = t.icon_small * 0.6;
    icon_frame(id, t)
        .relative()
        .tooltip(tip(tooltip))
        .child(button_icon(i, t))
        .child(div().absolute().top(t.hairline * 2.).right(t.hairline * 2.).child(icon(Icon::GoCorner, t.mute, t).size(corner)))
}

/// [`icon_action`] whose icon is in the accent when it can run: the one thing to do now
/// (the tag row's deploy once another tag is typed); dim with its reason when not.
pub fn icon_action_ready(id: impl Into<ElementId>, i: Icon, tooltip: impl Into<SharedString>, ready: bool, t: &Theme) -> Stateful<gpui::Div> {
    let child = icon(i, if ready { t.accent } else { t.dim }, t);
    icon_frame(id, t).tooltip(tip(tooltip)).child(child)
}

/// A notify dropdown's button (`docs/DESIGN.md` "Notifications"): the crossed bell when
/// nothing is ticked, the bell in the accent when anything is. No tooltip: its menu says
/// it all. It records where it was drawn in `at`, so a key opens the menu under it
/// ([`under`]). The caller adds the click.
pub fn notify_button(id: impl Into<ElementId>, ticked: bool, at: Rc<Cell<Option<Bounds<Pixels>>>>, t: &Theme) -> Stateful<gpui::Div> {
    let mark = if ticked { icon(Icon::Bell, t.accent, t) } else { button_icon(Icon::BellOff, t) };
    icon_frame(id, t)
        .relative()
        .child(mark)
        .child(gpui::canvas(move |b, _, _| at.set(Some(b)), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
}

/// Where a menu opened from a button drawn at `at` goes: under it (the window's corner
/// when it wasn't drawn yet).
pub fn under(at: Option<Bounds<Pixels>>) -> Point<Pixels> {
    at.map_or_else(Point::default, |b| b.bottom_left())
}

/// The square frame of an icon button, its icon brightening with it.
fn icon_frame(id: impl Into<ElementId>, t: &Theme) -> Stateful<gpui::Div> {
    let hover = t.hover;
    div()
        .id(id)
        .group(BUTTON)
        .w(t.icon_button)
        .h(t.icon_button)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(t.radius_s)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
}

/// [`icon_action`] for something on or off (wrap, follow, pause): framed in the accent
/// while on.
pub fn icon_toggle(id: impl Into<ElementId>, i: Icon, tooltip: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<gpui::Div> {
    let b = icon_action(id, i, tooltip, true, t).border_1();
    if on { b.border_color(t.accent).bg(t.accent_bg) } else { b.border_color(gpui::transparent_black()) }
}

/// Square icon-sized button with a tooltip.
pub fn icon_button(id: impl Into<ElementId>, i: Icon, tooltip: &'static str, t: &Theme) -> Stateful<gpui::Div> {
    let hover = t.hover;
    div()
        .id(id)
        .group(BUTTON)
        .w(t.icon_button)
        .h(t.icon_button)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(t.radius_s)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tip(tooltip))
        .child(button_icon(i, t))
}

/// An action's icon button, its icon `size` wide ([`Theme::icon`], or
/// [`Theme::icon_small`] in a dense list): bright when it can be used, dim when not (its
/// tooltip says why). The caller adds the click when enabled.
pub fn action_button(id: impl Into<ElementId>, i: Icon, size: Pixels, enabled: bool, tooltip: impl Into<SharedString>, t: &Theme) -> Stateful<gpui::Div> {
    let hover = t.hover;
    div()
        .id(id)
        .flex_none()
        .w(t.icon_button)
        .h(t.icon_button)
        .flex()
        .items_center()
        .justify_center()
        .rounded(t.radius_s)
        .when(enabled, |d| d.hover(move |s| s.bg(hover)).cursor_pointer())
        .tooltip(tip(tooltip))
        .child(icon(i, if enabled { t.fg } else { t.dim }, t).size(size))
}

/// A status dot (running, ready, unsaved…), filled or a ring, in a cell the size of an
/// icon so it lines up with them.
pub fn dot(color: Hsla, filled: bool, t: &Theme) -> gpui::Div {
    let d = div().size(t.size(7.)).rounded_full();
    let d = if filled { d.bg(color) } else { d.border_1().border_color(color) };
    div().size(t.icon).flex_none().flex().items_center().justify_center().child(d)
}

/// A message line: an icon, then the text, both in `color`.
pub fn icon_text(i: Icon, color: Hsla, text: impl Into<SharedString>, t: &Theme) -> gpui::Div {
    div().flex().items_center().gap_1().min_w_0().text_color(color).child(icon(i, color, t)).child(label(text))
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
            .child(icon(if open { Icon::Open } else { Icon::Expand }, t.dim, t).size(t.size(TREE_INDENT))),
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

/// A grouped block of a dashboard-like view (DESIGN: "a few grouped blocks"): its own
/// surface, a border, rounded corners; a header strip with its title (and count) on the
/// left and its `actions` on the right, over a soft rule; then `body`, padded. Panels
/// stand apart on the page's background.
pub fn panel(name: &str, title: impl Into<SharedString>, actions: Vec<AnyElement>, body: impl IntoElement, t: &Theme) -> gpui::Div {
    let (head, content) = (format!("{name}-head"), name.to_string());
    div()
        .debug_selector(move || content)
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .bg(t.bar)
        .border_1()
        .border_color(t.line)
        .rounded(t.radius_m)
        .overflow_hidden()
        .child(
            div()
                .debug_selector(move || head)
                .flex_none()
                .h(t.tab_h)
                .px_3()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .border_b_1()
                .border_color(t.line_soft)
                .child(label(title).flex_1().min_w_0().text_color(t.fg))
                .children(actions),
        )
        .child(div().px_1().py_1().flex().flex_col().min_w_0().child(body))
}

/// Fields of a summary: each a small caption over its value, side by side, wrapping to the
/// next line when the room runs out (a dashboard's facts: state, tag, revision…). The
/// caption reads as a label (small, dim, in capitals, as a table's header), the value as
/// data (the text's color unless it says otherwise).
pub fn fields(items: Vec<(SharedString, AnyElement)>, t: &Theme) -> gpui::Div {
    let mut row = div().flex().flex_wrap().gap_x_6().gap_y_1().px_2().py_1();
    for (caption, value) in items {
        row = row.child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(t.text_small).text_color(t.dim).child(caption.to_uppercase()))
                .child(div().min_h(t.row_h).flex().items_center().min_w_0().text_color(t.fg).child(value)),
        );
    }
    row
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
