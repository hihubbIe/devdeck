//! The chooser: it has the keys when it opens, ↑↓ move in the column that has them, ←→
//! and Tab go to the other, WHERE follows WHAT (keeping the same value), a click selects, a
//! double click on WHERE chooses, Enter or Open choose, Esc closes and the focus comes back;
//! its size never moves with the selection (R9.10).

use crate::common;

use common::host;
use devdeck_ui::chooser::{ChooserConfirm, ChooserDown, ChooserLeft, ChooserRight, ChooserUp, Column};
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{Choice, ChoiceRow, Chooser, ChooserSpec, open_chooser};
use gpui::{
    AnyWindowHandle, AppContext, Entity, FocusHandle, InteractiveElement, IntoElement, KeyBinding, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    TestAppContext, VisualTestContext, div,
};
use std::cell::RefCell;
use std::rc::Rc;

type Chosen = Rc<RefCell<Vec<(String, String)>>>;

fn spec(chosen: Chosen) -> ChooserSpec {
    let pulses = || vec![ChoiceRow::new("pulse:a", "default").detail("current"), ChoiceRow::new("pulse:b", "triage")];
    let home = |name: &str| std::iter::once(ChoiceRow::new("home", name.to_string()));
    ChooserSpec {
        title: "Open web in".into(),
        what_title: "WHAT".into(),
        where_title: "WHERE".into(),
        choices: vec![
            Choice { what: ChoiceRow::new("view", "web"), wheres: pulses() },
            Choice { what: ChoiceRow::new("terminal", "Terminal"), wheres: home("Terminals").chain(pulses()).collect() },
            Choice { what: ChoiceRow::new("agent:claude", "Claude Code"), wheres: home("Agents").chain(pulses()).collect() },
        ],
        confirm: "Open".into(),
        start_where: Some("pulse:b".into()),
        on_choose: Rc::new(move |what, place, _, _| chosen.borrow_mut().push((what, place))),
    }
}

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Chooser>, Chosen, FocusHandle) {
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("up", ChooserUp, Some("Chooser")),
            KeyBinding::new("down", ChooserDown, Some("Chooser")),
            KeyBinding::new("left", ChooserLeft, Some("Chooser")),
            KeyBinding::new("right", ChooserRight, Some("Chooser")),
            KeyBinding::new("enter", ChooserConfirm, Some("Chooser")),
            KeyBinding::new("escape", ModalClose, Some("Modal")),
            KeyBinding::new("tab", ModalNext, Some("Modal")),
            KeyBinding::new("shift-tab", ModalPrev, Some("Modal")),
        ])
    });
    let before: Rc<RefCell<Option<FocusHandle>>> = Rc::default();
    let b = before.clone();
    let window = host(cx, move |_, cx| {
        let f = b.borrow_mut().get_or_insert_with(|| cx.focus_handle()).clone();
        div().track_focus(&f).into_any_element()
    });
    cx.run_until_parked();
    let before = before.borrow().clone().expect("drawn");
    cx.update_window(window, |_, window, cx| before.focus(window, cx)).unwrap();
    let chosen: Chosen = Rc::default();
    let c = chosen.clone();
    let chooser = cx.update_window(window, |_, window, cx| open_chooser(spec(c), window, cx)).unwrap();
    cx.run_until_parked();
    (window, chooser, chosen, before)
}

fn selected(chooser: &Entity<Chooser>, cx: &TestAppContext) -> Option<(String, String)> {
    cx.read(|cx| chooser.read(cx).selected())
}

fn pair(what: &str, place: &str) -> Option<(String, String)> {
    Some((what.into(), place.into()))
}

#[gpui::test]
fn it_opens_with_the_keys_and_the_first_rows(cx: &mut TestAppContext) {
    let (window, chooser, _, _) = open(cx);
    let focus = cx.read(|cx| gpui::Focusable::focus_handle(chooser.read(cx), cx));
    assert!(cx.update_window(window, |_, window, _| focus.is_focused(window)).unwrap());
    assert_eq!(selected(&chooser, cx), pair("view", "pulse:b"), "the WHERE asked for first");
    assert_eq!(cx.read(|cx| chooser.read(cx).column), Column::What);
    let mut vcx = VisualTestContext::from_window(window, cx);
    assert!(vcx.debug_bounds("modal-panel").is_some(), "in the modal frame, over its backdrop");
}

#[gpui::test]
fn the_keys_move_in_a_column_and_between_them(cx: &mut TestAppContext) {
    let (window, chooser, chosen, _) = open(cx);
    // ↓ on WHAT: Terminal, its WHERE keeps the same pulse.
    cx.simulate_keystrokes(window, "down");
    assert_eq!(selected(&chooser, cx), pair("terminal", "pulse:b"));
    // → then ↑↑: to its first, Terminals (↑ stops there).
    cx.simulate_keystrokes(window, "right up up up");
    assert_eq!(selected(&chooser, cx), pair("terminal", "home"));
    // Tab back to WHAT, ↓: Claude Code keeps "home" (its own: Agents).
    cx.simulate_keystrokes(window, "tab down");
    assert_eq!(selected(&chooser, cx), pair("agent:claude", "home"));
    // ↑ twice on WHAT: the view has no "home", so its first.
    cx.simulate_keystrokes(window, "left up up");
    assert_eq!(selected(&chooser, cx), pair("view", "pulse:a"));
    cx.simulate_keystrokes(window, "enter");
    assert_eq!(*chosen.borrow(), [("view".to_string(), "pulse:a".to_string())]);
    assert!(!cx.update(|cx| devdeck_ui::overlay::any_modal(cx)), "choosing closes it");
}

#[gpui::test]
fn a_click_selects_a_double_click_chooses(cx: &mut TestAppContext) {
    let (window, chooser, chosen, _) = open(cx);
    let mut vcx = VisualTestContext::from_window(window, cx);
    let terminal = vcx.debug_bounds("chooser-what-1").expect("Terminal");
    vcx.simulate_click(terminal.center(), Modifiers::none());
    assert_eq!(selected(&chooser, cx), pair("terminal", "pulse:b"));
    let mut vcx = VisualTestContext::from_window(window, cx);
    let home = vcx.debug_bounds("chooser-where-0").expect("Terminals");
    vcx.simulate_click(home.center(), Modifiers::none());
    assert_eq!(selected(&chooser, cx), pair("terminal", "home"));
    assert_eq!(cx.read(|cx| chooser.read(cx).column), Column::Where, "its column has the keys");
    assert!(chosen.borrow().is_empty(), "a click only selects");
    let mut vcx = VisualTestContext::from_window(window, cx);
    let at = vcx.debug_bounds("chooser-where-1").expect("default").center();
    for n in 1..=2 {
        vcx.simulate_event(MouseDownEvent { position: at, button: MouseButton::Left, modifiers: Modifiers::none(), click_count: n, first_mouse: false });
        vcx.simulate_event(MouseUpEvent { position: at, button: MouseButton::Left, modifiers: Modifiers::none(), click_count: n });
    }
    assert_eq!(*chosen.borrow(), [("terminal".to_string(), "pulse:a".to_string())]);
}

#[gpui::test]
fn open_chooses_cancel_and_esc_close_and_give_the_focus_back(cx: &mut TestAppContext) {
    let (window, _, chosen, before) = open(cx);
    let mut vcx = VisualTestContext::from_window(window, cx);
    let open_button = vcx.debug_bounds("chooser-confirm").expect("Open");
    vcx.simulate_click(open_button.center(), Modifiers::none());
    assert_eq!(*chosen.borrow(), [("view".to_string(), "pulse:b".to_string())]);
    let (window, _, chosen, before2) = open(cx);
    cx.simulate_keystrokes(window, "escape");
    assert!(chosen.borrow().is_empty());
    assert!(!cx.update(|cx| devdeck_ui::overlay::any_modal(cx)), "Esc closes it");
    assert!(cx.update_window(window, |_, window, _| before2.is_focused(window)).unwrap(), "the focus is back");
    let _ = before;
}

#[gpui::test]
fn its_size_never_moves_with_the_selection(cx: &mut TestAppContext) {
    let (window, _, _, _) = open(cx);
    let mut vcx = VisualTestContext::from_window(window, cx);
    let panel = vcx.debug_bounds("modal-panel").expect("drawn");
    // Terminal's WHERE has one more row than the view's.
    cx.simulate_keystrokes(window, "down");
    let mut vcx = VisualTestContext::from_window(window, cx);
    assert_eq!(vcx.debug_bounds("modal-panel"), Some(panel));
    cx.simulate_keystrokes(window, "right down");
    let mut vcx = VisualTestContext::from_window(window, cx);
    assert_eq!(vcx.debug_bounds("modal-panel"), Some(panel));
}
