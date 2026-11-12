//! A view's own list of places (Settings' categories): one row each, a label and an
//! optional count, the selected one brighter while the list has the keys. Rows keep their
//! size whatever their state (R9.10); a long label truncates (R4.4).

use crate::label;
use crate::theme::Theme;
use gpui::{App, ElementId, InteractiveElement, ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div, prelude::*};
use std::rc::Rc;

/// One row of a [`nav_list`].
#[derive(Clone, Debug, PartialEq)]
pub struct NavItem {
    pub label: SharedString,
    /// Shown dim at its end (a search's matches).
    pub count: Option<usize>,
}

impl NavItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), count: None }
    }

    pub fn count(mut self, n: usize) -> Self {
        self.count = Some(n);
        self
    }
}

/// The rows of `items` under `id` (each `<id>-<i>` for tests), `selected` drawn in the
/// selection's color when `keys` (the list has them), else a dimmer one; a click calls
/// `on_pick` with the row's index.
pub fn nav_list(id: &'static str, items: &[NavItem], selected: Option<usize>, keys: bool, t: &Theme, on_pick: impl Fn(usize, &mut Window, &mut App) + 'static) -> Stateful<gpui::Div> {
    let on_pick = Rc::new(on_pick);
    let mut list = div().id(id).flex().flex_col().min_w_0();
    for (i, item) in items.iter().enumerate() {
        let on = selected == Some(i);
        let pick = on_pick.clone();
        let name = format!("{id}-{i}");
        list = list.child(
            div()
                .id(ElementId::NamedInteger(id.into(), i as u64))
                .debug_selector(move || name.clone())
                .h(t.row_h)
                .flex_none()
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .rounded(t.radius_s)
                .cursor_pointer()
                // The selected row keeps its color under the pointer.
                .when(!on, |d| d.hover(|s| s.bg(t.hover)))
                .when(on, |d| d.bg(if keys { t.selection } else { t.line }))
                .on_click(move |_, window, cx| pick(i, window, cx))
                .child(label(item.label.clone()).flex_1().text_color(if on { t.hi } else { t.fg }))
                .children(item.count.map(|n| div().flex_none().text_color(t.mute).child(n.to_string()))),
        );
    }
    list
}
