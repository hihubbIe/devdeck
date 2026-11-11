//! The picker: its search has the keys when it opens, typing and the kind chips narrow
//! the options, up/down/enter pick, a click picks, Esc closes; what's typed is an option
//! when the caller says so. The table: its columns line up whatever's in the cells.

use crate::common;

use common::host;
use devdeck_ui::overlay::ModalClose;
use devdeck_ui::picker::{PickerDown, PickerUp};
use devdeck_ui::{Column, PickGroup, PickOption, Picker, PickerSpec, open_picker, table_head, table_row};
use gpui::{AnyWindowHandle, AppContext, Entity, Focusable, InteractiveElement, IntoElement, KeyBinding, Modifiers, ParentElement, Styled, TestAppContext, VisualTestContext, div};
use std::cell::RefCell;
use std::rc::Rc;

fn spec(picked: Rc<RefCell<Vec<String>>>, typed: bool) -> PickerSpec {
    PickerSpec {
        title: "Deploy graphservice".into(),
        placeholder: "search branches and tags".into(),
        kinds: vec!["branch", "tag"],
        groups: vec![
            PickGroup { label: "ON OTHER ENVS".into(), options: vec![PickOption::new("1.62.8").chip("dev31").kind("tag"), PickOption::new("main").chip("dev33").kind("branch")] },
            PickGroup { label: "BRANCHES".into(), options: vec![PickOption::new("feature/geo-index").detail("1 d ago · j.doe").kind("branch")] },
        ],
        typed: typed.then(|| "as typed".into()),
        on_pick: Rc::new(move |v, _, _| picked.borrow_mut().push(v)),
    }
}

fn open(cx: &mut TestAppContext, typed: bool) -> (AnyWindowHandle, Entity<Picker>, Rc<RefCell<Vec<String>>>) {
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("down", PickerDown, Some("Picker")),
            KeyBinding::new("up", PickerUp, Some("Picker")),
            KeyBinding::new("escape", ModalClose, Some("Modal")),
            KeyBinding::new("enter", devdeck_ui::text_input::Submit, Some("TextInput")),
            KeyBinding::new("escape", devdeck_ui::text_input::Cancel, Some("TextInput")),
        ])
    });
    let window = host(cx, |_, _| div().into_any_element());
    let picked = Rc::new(RefCell::new(Vec::new()));
    let p = picked.clone();
    let picker = cx.update_window(window, |_, window, cx| open_picker(spec(p, typed), window, cx)).unwrap();
    cx.run_until_parked();
    (window, picker, picked)
}

fn labels(picker: &Entity<Picker>, cx: &TestAppContext) -> Vec<String> {
    cx.read(|cx| picker.read(cx).shown(cx).into_iter().flat_map(|g| g.options).map(|o| o.value).collect())
}

#[gpui::test]
fn the_search_has_the_keys_and_narrows_as_one_types(cx: &mut TestAppContext) {
    let (window, picker, _) = open(cx, false);
    let input = cx.read(|cx| picker.read(cx).input.read(cx).focus_handle(cx));
    assert!(cx.update_window(window, |_, window, _| input.is_focused(window)).unwrap(), "typing goes to the search at once");
    assert_eq!(labels(&picker, cx), ["1.62.8", "main", "feature/geo-index"]);
    cx.simulate_input(window, "geo");
    assert_eq!(labels(&picker, cx), ["feature/geo-index"]);
}

#[gpui::test]
fn a_kind_chip_narrows_to_that_kind(cx: &mut TestAppContext) {
    let (window, picker, _) = open(cx, false);
    let mut vcx = VisualTestContext::from_window(window, cx);
    let tag = vcx.debug_bounds("pick-kind-tag").expect("drawn");
    vcx.simulate_click(tag.center(), Modifiers::none());
    assert_eq!(labels(&picker, cx), ["1.62.8"]);
}

#[gpui::test]
fn the_keys_choose_and_enter_picks(cx: &mut TestAppContext) {
    let (window, _, picked) = open(cx, false);
    cx.simulate_keystrokes(window, "down down up down enter");
    assert_eq!(*picked.borrow(), ["feature/geo-index"]);
    assert!(!cx.update(|cx| devdeck_ui::overlay::any_modal(cx)), "picking closes it");
}

#[gpui::test]
fn a_click_picks_and_esc_closes(cx: &mut TestAppContext) {
    let (window, _, picked) = open(cx, false);
    let mut vcx = VisualTestContext::from_window(window, cx);
    let row = vcx.debug_bounds("pick-1").expect("drawn");
    vcx.simulate_click(row.center(), Modifiers::none());
    assert_eq!(*picked.borrow(), ["main"]);
    let (window, _, picked) = open(cx, false);
    cx.simulate_keystrokes(window, "escape");
    assert!(picked.borrow().is_empty());
    assert!(!cx.update(|cx| devdeck_ui::overlay::any_modal(cx)), "Esc closes it");
}

#[gpui::test]
fn what_is_typed_is_an_option_when_asked(cx: &mut TestAppContext) {
    let (window, picker, picked) = open(cx, true);
    cx.simulate_input(window, "lan-301");
    assert_eq!(labels(&picker, cx), ["lan-301"], "first, nothing else matching");
    cx.simulate_keystrokes(window, "enter");
    assert_eq!(*picked.borrow(), ["lan-301"]);
}

#[gpui::test]
fn a_tables_columns_line_up_whatever_the_cells_hold(cx: &mut TestAppContext) {
    const COLS: [Column; 3] = [Column::rest("POD"), Column::fixed("STATE", 90.), Column::fixed("RESTARTS", 64.).right()];
    let window = host(cx, |_, cx| {
        let t = devdeck_ui::theme(cx).clone();
        let cell = |s: &'static str, id: &'static str| div().debug_selector(move || id.into()).child(s).into_any_element();
        div()
            .w(t.size(600.))
            .child(table_head(&COLS, &t))
            .child(table_row("a", &COLS, vec![cell("graphservice-7d9f-x2k4q", "a0"), cell("ready", "a1"), cell("", "a2")], &t))
            .child(table_row("b", &COLS, vec![cell("g-1", "b0"), cell("starting", "b1"), cell("↻ 12", "b2")], &t))
            .into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    let (a1, b1) = (vcx.debug_bounds("a1").unwrap(), vcx.debug_bounds("b1").unwrap());
    assert_eq!(a1.origin.x, b1.origin.x, "the state column starts at the same place on every row");
    let (a0, b0) = (vcx.debug_bounds("a0").unwrap(), vcx.debug_bounds("b0").unwrap());
    assert_eq!(a0.origin.x, b0.origin.x);
}
