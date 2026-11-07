//! The single-line text input on its own: typing (through the same input handler IME
//! uses), caret and selection by keys and by mouse, deleting, undo, the events it
//! reports, scrolling to keep the caret visible, characters that are one glyph but
//! several code points. Keys are bound here as the app binds them (its registry), in the
//! `TextInput` context; the clipboard is left to the app's tests.

mod common;

use common::host;
use devdeck_ui::text_input::*;
use devdeck_ui::{InputEvent, TextInput, logical};
use gpui::{
    AnyWindowHandle, AppContext, Entity, Focusable, InteractiveElement, IntoElement, KeyBinding, Modifiers, MouseButton, MouseDownEvent,
    ParentElement, Styled, TestAppContext, VisualTestContext, div, point,
};
use std::cell::RefCell;
use std::rc::Rc;

struct Field {
    window: AnyWindowHandle,
    input: Entity<TextInput>,
    events: Rc<RefCell<Vec<InputEvent>>>,
    /// Esc and Enter seen by the element around the input (they keep bubbling).
    outer: Rc<RefCell<Vec<&'static str>>>,
}

impl Field {
    fn text(&self, cx: &TestAppContext) -> String {
        cx.read(|cx| self.input.read(cx).text().to_string())
    }

    fn keys(&self, cx: &mut TestAppContext, keys: &str) {
        cx.simulate_keystrokes(self.window, keys);
        cx.run_until_parked();
    }

    fn type_text(&self, cx: &mut TestAppContext, text: &str) {
        cx.simulate_input(self.window, text);
        cx.run_until_parked();
    }

    fn set(&self, cx: &mut TestAppContext, text: &str) {
        cx.update(|cx| self.input.update(cx, |i, cx| i.set_text(text, cx)));
        cx.run_until_parked();
    }
}

/// An input `width` px wide in a host window, focused, with the app's keys for it.
fn field(cx: &mut TestAppContext, width: f32) -> Field {
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("backspace", Backspace, Some("TextInput")),
            KeyBinding::new("delete", Delete, Some("TextInput")),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, Some("TextInput")),
            KeyBinding::new("left", Left, Some("TextInput")),
            KeyBinding::new("right", Right, Some("TextInput")),
            KeyBinding::new("shift-left", SelectLeft, Some("TextInput")),
            KeyBinding::new("shift-right", SelectRight, Some("TextInput")),
            KeyBinding::new("ctrl-left", WordLeft, Some("TextInput")),
            KeyBinding::new("ctrl-right", WordRight, Some("TextInput")),
            KeyBinding::new("ctrl-shift-left", SelectWordLeft, Some("TextInput")),
            KeyBinding::new("ctrl-shift-right", SelectWordRight, Some("TextInput")),
            KeyBinding::new("home", Home, Some("TextInput")),
            KeyBinding::new("end", End, Some("TextInput")),
            KeyBinding::new("shift-home", SelectHome, Some("TextInput")),
            KeyBinding::new("shift-end", SelectEnd, Some("TextInput")),
            KeyBinding::new("ctrl-a", SelectAll, Some("TextInput")),
            KeyBinding::new("ctrl-z", Undo, Some("TextInput")),
            KeyBinding::new("ctrl-shift-z", Redo, Some("TextInput")),
            KeyBinding::new("enter", Submit, Some("TextInput")),
            KeyBinding::new("escape", Cancel, Some("TextInput")),
        ])
    });
    let input = cx.new(|cx| TextInput::new("filter", cx));
    let outer = Rc::new(RefCell::new(Vec::new()));
    let (shown, seen_submit, seen_cancel) = (input.clone(), outer.clone(), outer.clone());
    let window = host(cx, move |_, _| {
        let (s, c) = (seen_submit.clone(), seen_cancel.clone());
        div()
            .p(logical(20.))
            .on_action(move |_: &Submit, _, _| s.borrow_mut().push("submit"))
            .on_action(move |_: &Cancel, _, _| c.borrow_mut().push("cancel"))
            .child(div().w(logical(width)).debug_selector(|| "field".into()).child(shown.clone()))
            .into_any_element()
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|cx| cx.subscribe(&input, move |_, e: &InputEvent, _| sink.borrow_mut().push(e.clone())).detach());
    let focus = cx.read(|cx| input.read(cx).focus_handle(cx));
    cx.update_window(window, |_, window, cx| window.focus(&focus, cx)).unwrap();
    cx.run_until_parked();
    Field { window, input, events, outer }
}

#[gpui::test]
fn typing_inserts_text_and_reports_each_change(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    f.type_text(cx, "héllo 世界");
    assert_eq!(f.text(cx), "héllo 世界");
    assert_eq!(f.events.borrow().last(), Some(&InputEvent::Changed("héllo 世界".into())));
}

#[gpui::test]
fn keys_move_the_caret_and_select(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    f.set(cx, "one two three");
    // To the start, select the first word, replace it.
    f.keys(cx, "home ctrl-shift-right");
    f.type_text(cx, "ONE");
    assert_eq!(f.text(cx), "ONE two three");
    // From the end, one char left, select to the start: everything but the last char.
    f.keys(cx, "end left shift-home");
    f.type_text(cx, "x");
    assert_eq!(f.text(cx), "xe");
    // Select all replaces all.
    f.keys(cx, "ctrl-a");
    f.type_text(cx, "new");
    assert_eq!(f.text(cx), "new");
}

#[gpui::test]
fn deleting_by_character_and_by_word(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    f.set(cx, "alpha beta");
    f.keys(cx, "end backspace");
    assert_eq!(f.text(cx), "alpha bet");
    f.keys(cx, "ctrl-backspace");
    assert_eq!(f.text(cx), "alpha ");
    f.keys(cx, "home delete");
    assert_eq!(f.text(cx), "lpha ");
}

#[gpui::test]
fn undo_and_redo(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    f.type_text(cx, "abc");
    f.keys(cx, "backspace");
    assert_eq!(f.text(cx), "ab");
    f.keys(cx, "ctrl-z");
    assert_eq!(f.text(cx), "abc");
    f.keys(cx, "ctrl-shift-z");
    assert_eq!(f.text(cx), "ab");
}

#[gpui::test]
fn enter_and_escape_are_reported_and_keep_bubbling(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    f.type_text(cx, "query");
    f.keys(cx, "enter");
    f.keys(cx, "escape");
    let events = f.events.borrow();
    assert_eq!(events[events.len() - 2..], [InputEvent::Submit("query".into()), InputEvent::Cancel]);
    assert_eq!(*f.outer.borrow(), ["submit", "cancel"], "a dialog around the input still sees them");
}

#[gpui::test]
fn a_moving_caret_or_selection_alone_is_not_a_change(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    f.set(cx, "abc");
    let before = f.events.borrow().len();
    f.keys(cx, "home right shift-end left");
    assert_eq!(f.events.borrow().len(), before);
}

#[gpui::test]
fn a_glyph_made_of_several_code_points_is_never_split(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    // e + combining acute, a family emoji (joined with zero-width joiners), CJK.
    f.set(cx, "ae\u{301}👨‍👩‍👧中");
    f.keys(cx, "end backspace");
    assert_eq!(f.text(cx), "ae\u{301}👨‍👩‍👧");
    f.keys(cx, "backspace");
    assert_eq!(f.text(cx), "ae\u{301}");
    f.keys(cx, "left delete");
    assert_eq!(f.text(cx), "a");
}

#[gpui::test]
fn a_click_places_the_caret_and_a_drag_selects(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    f.set(cx, "abcdef");
    let mut vcx = VisualTestContext::from_window(f.window, cx);
    let b = vcx.debug_bounds("field").expect("drawn");
    let (left, right, y) = (b.left() + logical(4.), b.right() - logical(4.), b.center().y);
    // A click before the text puts the caret at its start.
    vcx.simulate_click(point(left, y), Modifiers::none());
    vcx.simulate_input("X");
    assert_eq!(vcx.read(|cx| f.input.read(cx).text().to_string()), "Xabcdef");
    // A drag across everything selects it.
    vcx.simulate_mouse_down(point(left, y), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(point(right, y), MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(point(right, y), MouseButton::Left, Modifiers::none());
    vcx.simulate_input("Y");
    assert_eq!(vcx.read(|cx| f.input.read(cx).text().to_string()), "Y");
}

#[gpui::test]
fn a_click_focuses_the_input(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    let focus = cx.read(|cx| f.input.read(cx).focus_handle(cx));
    cx.update_window(f.window, |_, window, cx| window.blur(cx)).unwrap();
    let mut vcx = VisualTestContext::from_window(f.window, cx);
    let b = vcx.debug_bounds("field").expect("drawn");
    vcx.simulate_click(b.center(), Modifiers::none());
    assert!(vcx.update(|window, _| focus.is_focused(window)));
}

#[gpui::test]
fn a_double_click_selects_a_word(cx: &mut TestAppContext) {
    let f = field(cx, 300.);
    f.set(cx, "one");
    let mut vcx = VisualTestContext::from_window(f.window, cx);
    let b = vcx.debug_bounds("field").expect("drawn");
    let at = point(b.left() + logical(12.), b.center().y);
    vcx.simulate_event(MouseDownEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::none(), click_count: 2, first_mouse: false });
    vcx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
    vcx.simulate_input("two");
    assert_eq!(vcx.read(|cx| f.input.read(cx).text().to_string()), "two");
}

#[gpui::test]
fn a_long_line_scrolls_to_keep_the_caret_visible(cx: &mut TestAppContext) {
    let f = field(cx, 80.);
    f.set(cx, &"wide text that does not fit ".repeat(4));
    f.keys(cx, "end");
    assert!(cx.read(|cx| f.input.read(cx).scroll_x()) > logical(0.), "scrolled to the end");
    f.keys(cx, "home");
    assert_eq!(cx.read(|cx| f.input.read(cx).scroll_x()), logical(0.), "back to the start");
}

#[gpui::test]
fn composing_text_marks_it_until_committed(cx: &mut TestAppContext) {
    use gpui::EntityInputHandler;
    let f = field(cx, 300.);
    // An input method composing "ni", then committing 你 in its place.
    cx.update_window(f.window, |_, window, cx| {
        f.input.update(cx, |i, cx| i.replace_and_mark_text_in_range(None, "ni", None, window, cx));
        assert_eq!(f.input.update(cx, |i, cx| i.marked_text_range(window, cx)), Some(0..2));
        f.input.update(cx, |i, cx| i.replace_text_in_range(None, "你", window, cx));
        assert_eq!(f.input.update(cx, |i, cx| i.marked_text_range(window, cx)), None);
    })
    .unwrap();
    assert_eq!(f.text(cx), "你");
}
