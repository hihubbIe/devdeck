//! Single-line text input (filters, palette, tag editor). Adapted from GPUI's
//! `examples/input.rs`; all editing goes through the tested [`TextBuffer`].
//!
//! - Keys live in the `TextInput` key context, so app shortcuts bound with
//!   `"Panes && !TextInput"` never fire while typing.
//! - The line scrolls horizontally to keep the caret visible and never wraps (R4.4).
//! - The caret doesn't blink, so an idle focused input doesn't render (R1.3).
//! - Enter and Esc are reported as [`InputEvent`]s for the owner to act on.

use crate::text_buffer::TextBuffer;
use crate::theme::theme;
use gpui::{
    App, Bounds, ClipboardItem, Context, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Render, SharedString, ShapedLine, Style,
    TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, point, prelude::*, px, relative, size,
};
use std::ops::Range;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        DeleteWordLeft,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        Home,
        End,
        SelectHome,
        SelectEnd,
        SelectAll,
        Copy,
        Cut,
        Paste,
        Undo,
        Redo,
        Submit,
        Cancel,
    ]
);

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    Changed(String),
    Submit(String),
    Cancel,
}

pub struct TextInput {
    focus: FocusHandle,
    buf: TextBuffer,
    placeholder: SharedString,
    scroll_x: Pixels,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    selecting: bool,
}

impl EventEmitter<InputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TextInput {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            buf: TextBuffer::new(""),
            placeholder: placeholder.into(),
            scroll_x: Pixels::ZERO,
            last_layout: None,
            last_bounds: None,
            selecting: false,
        }
    }

    pub fn text(&self) -> &str {
        self.buf.text()
    }

    /// How far the line is scrolled to keep the caret visible.
    pub fn scroll_x(&self) -> Pixels {
        self.scroll_x
    }

    pub fn set_text(&mut self, s: &str, cx: &mut Context<Self>) {
        self.buf.set_text(s);
        self.changed(cx);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Changed(self.buf.text().to_string()));
        cx.notify();
    }

    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut TextBuffer)) {
        let before = self.buf.text().to_string();
        f(&mut self.buf);
        if self.buf.text() != before {
            self.changed(cx);
        } else {
            cx.notify();
        }
    }

    fn index_for_x(&self, x: Pixels) -> usize {
        let (Some(layout), Some(bounds)) = (&self.last_layout, self.last_bounds) else { return 0 };
        if self.buf.text().is_empty() {
            return 0;
        }
        layout.closest_index_for_x(x - bounds.left() + self.scroll_x)
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let i = self.index_for_x(e.position.x);
        if e.click_count == 2 {
            self.buf.select(self.buf.word_at(i));
        } else {
            self.selecting = true;
            self.buf.move_to(i, e.modifiers.shift);
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn on_mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            let i = self.index_for_x(e.position.x);
            self.buf.move_to(i, true);
            cx.notify();
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.buf.has_selection() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.selected_text().to_string()));
        }
    }

    fn cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        if self.buf.has_selection() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.buf.selected_text().to_string()));
            self.edit(cx, |b| b.insert(""));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.edit(cx, |b| b.insert(&text));
        }
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(&mut self, r: Range<usize>, actual: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        let (a, b) = (self.buf.utf16_to_byte(r.start), self.buf.utf16_to_byte(r.end));
        actual.replace(self.buf.byte_to_utf16(a)..self.buf.byte_to_utf16(b));
        Some(self.buf.text()[a..b].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        let s = self.buf.selection();
        Some(UTF16Selection {
            range: self.buf.byte_to_utf16(s.start)..self.buf.byte_to_utf16(s.end),
            reversed: self.buf.caret() < s.end,
        })
    }

    fn set_selected_text_range(&mut self, r: Range<usize>, _: &mut Window, cx: &mut Context<Self>) {
        let r = self.buf.utf16_to_byte(r.start)..self.buf.utf16_to_byte(r.end);
        self.buf.select(r);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(self.buf.byte_to_utf16(self.buf.text().len()))
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.buf.marked().map(|m| self.buf.byte_to_utf16(m.start)..self.buf.byte_to_utf16(m.end))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.buf.unmark();
        cx.notify();
    }

    fn replace_text_in_range(&mut self, r: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        let range = r
            .map(|r| self.buf.utf16_to_byte(r.start)..self.buf.utf16_to_byte(r.end))
            .or(self.buf.marked())
            .unwrap_or(self.buf.selection());
        self.edit(cx, |b| b.replace(range, text));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        r: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = r
            .map(|r| self.buf.utf16_to_byte(r.start)..self.buf.utf16_to_byte(r.end))
            .or(self.buf.marked())
            .unwrap_or(self.buf.selection());
        // `selected` is in UTF-16 units relative to `text`.
        let selected = selected.map(|s| {
            let to_byte = |u: usize| text.char_indices().scan(0, |n, (i, c)| {
                let at = *n;
                *n += c.len_utf16();
                Some((at, i))
            }).find(|(at, _)| *at >= u).map_or(text.len(), |(_, i)| i);
            to_byte(s.start)..to_byte(s.end)
        });
        self.edit(cx, |b| b.replace_and_mark(range, text, selected));
    }

    fn bounds_for_range(&mut self, r: Range<usize>, bounds: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let (a, b) = (self.buf.utf16_to_byte(r.start), self.buf.utf16_to_byte(r.end));
        Some(Bounds::from_corners(
            point(bounds.left() + layout.x_for_index(a) - self.scroll_x, bounds.top()),
            point(bounds.left() + layout.x_for_index(b) - self.scroll_x, bounds.bottom()),
        ))
    }

    fn character_index_for_point(&mut self, p: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        let local = bounds.localize(&p)?;
        Some(self.buf.byte_to_utf16(layout.closest_index_for_x(local.x + self.scroll_x)))
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let focused_border = t.accent;
        div()
            .id("text-input")
            .key_context("TextInput")
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .flex()
            .items_center()
            .min_w_0()
            .h(t.button_h)
            .px_2()
            .bg(t.bg)
            .border_1()
            .border_color(t.line)
            .rounded(t.radius_m)
            .focus(move |s| s.border_color(focused_border))
            .on_action(cx.listener(|this, _: &Backspace, _, cx| this.edit(cx, TextBuffer::backspace)))
            .on_action(cx.listener(|this, _: &Delete, _, cx| this.edit(cx, TextBuffer::delete)))
            .on_action(cx.listener(|this, _: &DeleteWordLeft, _, cx| this.edit(cx, TextBuffer::delete_word_left)))
            .on_action(cx.listener(|this, _: &Left, _, cx| this.edit(cx, |b| b.left(false))))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.edit(cx, |b| b.right(false))))
            .on_action(cx.listener(|this, _: &SelectLeft, _, cx| this.edit(cx, |b| b.left(true))))
            .on_action(cx.listener(|this, _: &SelectRight, _, cx| this.edit(cx, |b| b.right(true))))
            .on_action(cx.listener(|this, _: &WordLeft, _, cx| this.edit(cx, |b| b.word_left(false))))
            .on_action(cx.listener(|this, _: &WordRight, _, cx| this.edit(cx, |b| b.word_right(false))))
            .on_action(cx.listener(|this, _: &SelectWordLeft, _, cx| this.edit(cx, |b| b.word_left(true))))
            .on_action(cx.listener(|this, _: &SelectWordRight, _, cx| this.edit(cx, |b| b.word_right(true))))
            .on_action(cx.listener(|this, _: &Home, _, cx| this.edit(cx, |b| b.home(false))))
            .on_action(cx.listener(|this, _: &End, _, cx| this.edit(cx, |b| b.end(false))))
            .on_action(cx.listener(|this, _: &SelectHome, _, cx| this.edit(cx, |b| b.home(true))))
            .on_action(cx.listener(|this, _: &SelectEnd, _, cx| this.edit(cx, |b| b.end(true))))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.edit(cx, TextBuffer::select_all)))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.edit(cx, TextBuffer::undo)))
            .on_action(cx.listener(|this, _: &Redo, _, cx| this.edit(cx, TextBuffer::redo)))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            // Enter and Esc are reported, then keep bubbling so a menu, modal or the
            // settings view around the input can react too (Esc closes a dialog).
            .on_action(cx.listener(|this, _: &Submit, _, cx| {
                cx.emit(InputEvent::Submit(this.buf.text().to_string()));
                cx.propagate();
            }))
            .on_action(cx.listener(|_, _: &Cancel, _, cx| {
                cx.emit(InputEvent::Cancel);
                cx.propagate();
            }))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(LineElement { input: cx.entity() })
    }
}

struct LineElement {
    input: Entity<TextInput>,
}

struct LinePrepaint {
    line: ShapedLine,
    caret: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    scroll_x: Pixels,
}

impl IntoElement for LineElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for LineElement {
    type RequestLayoutState = ();
    type PrepaintState = LinePrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&gpui::InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        style.min_size.width = Pixels::ZERO.into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> LinePrepaint {
        let t = theme(cx).clone();
        let input = self.input.read(cx);
        let style = window.text_style();
        let empty = input.buf.text().is_empty();
        let (text, color): (SharedString, _) =
            if empty { (input.placeholder.clone(), t.dim) } else { (input.buf.text().to_string().into(), style.color) };
        let run = TextRun { len: text.len(), font: style.font(), color, background_color: None, underline: None, strikethrough: None };
        let runs = match input.buf.marked().filter(|_| !empty) {
            Some(m) => vec![
                TextRun { len: m.start, ..run.clone() },
                TextRun {
                    len: m.end - m.start,
                    underline: Some(UnderlineStyle { color: Some(run.color), thickness: px(1.), wavy: false }),
                    ..run.clone()
                },
                TextRun { len: text.len() - m.end, ..run },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(text, font_size, &runs, None);

        // Keep the caret visible: scroll just enough, never past the text's end.
        let caret_w = px(2.);
        let caret_x = if empty { Pixels::ZERO } else { line.x_for_index(input.buf.caret()) };
        let width = bounds.size.width;
        let mut scroll_x = input.scroll_x;
        if caret_x < scroll_x {
            scroll_x = caret_x;
        } else if caret_x + caret_w > scroll_x + width {
            scroll_x = caret_x + caret_w - width;
        }
        let max_scroll = (line.width + caret_w - width).max(Pixels::ZERO);
        scroll_x = scroll_x.clamp(Pixels::ZERO, max_scroll);

        let sel = input.buf.selection();
        let left = bounds.left() - scroll_x;
        let (selection, caret) = if sel.is_empty() || empty {
            (None, Some(fill(Bounds::new(point(left + caret_x, bounds.top()), size(caret_w, bounds.size.height)), t.caret)))
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(left + line.x_for_index(sel.start), bounds.top()),
                        point(left + line.x_for_index(sel.end), bounds.bottom()),
                    ),
                    t.selection,
                )),
                None,
            )
        };
        LinePrepaint { line, caret, selection, scroll_x }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut LinePrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let scroll_x = prepaint.scroll_x;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection);
            }
            let origin = point(bounds.left() - scroll_x, bounds.top());
            let _ = prepaint.line.paint(origin, window.line_height(), gpui::TextAlign::Left, None, window, cx);
            if focus.is_focused(window)
                && let Some(caret) = prepaint.caret.take()
            {
                window.paint_quad(caret);
            }
        });
        let line = prepaint.line.clone();
        self.input.update(cx, |input, _| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
            input.scroll_x = scroll_x;
        });
    }
}
