//! The nav list: a click picks a row, its count shows, rows keep their size whatever
//! is selected or has the keys (R9.10), a long label truncates (R4.4).

use crate::common;

use common::host;
use devdeck_ui::{NavItem, nav_list};
use gpui::{IntoElement, Modifiers, ParentElement, Styled, TestAppContext, VisualTestContext, div};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn items() -> Vec<NavItem> {
    vec![NavItem::new("Agents"), NavItem::new("Appearance").count(2), NavItem::new("a category with a very long name that never wraps in the list")]
}

#[gpui::test]
fn a_click_picks_and_rows_keep_their_size(cx: &mut TestAppContext) {
    let picked = Rc::new(RefCell::new(Vec::new()));
    let state = Rc::new(Cell::new((Some(0usize), true)));
    let (p, s) = (picked.clone(), state.clone());
    let window = host(cx, move |_, cx| {
        let t = devdeck_ui::theme(cx).clone();
        let p = p.clone();
        let (selected, keys) = s.get();
        div().w(t.size(160.)).child(nav_list("nav", &items(), selected, keys, &t, move |i, _, _| p.borrow_mut().push(i))).into_any_element()
    });
    let mut vcx = VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    let rows: Vec<_> = (0..3).map(|i| vcx.debug_bounds(Box::leak(format!("nav-{i}").into_boxed_str())).expect("drawn")).collect();
    assert!(rows.iter().all(|r| r.size.height == rows[0].size.height), "one height, the long one too: {rows:?}");
    assert!(vcx.debug_bounds("nav-1").is_some_and(|r| r.size.width == rows[0].size.width), "the counted row as wide as the others");
    vcx.simulate_click(rows[1].center(), Modifiers::none());
    assert_eq!(*picked.borrow(), [1]);
    // Another selection, the keys elsewhere: nothing moves.
    state.set((Some(2), false));
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let after: Vec<_> = (0..3).map(|i| vcx.debug_bounds(Box::leak(format!("nav-{i}").into_boxed_str())).unwrap()).collect();
    assert_eq!(rows, after);
}
