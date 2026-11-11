//! A table: a header and rows on fixed columns. Each column has a width (in theme units,
//! following the UI scale) or takes what's left; a cell truncates, an empty one keeps its
//! room, so nothing shifts from row to row. A row can hold more under it, opened in place
//! ([`table_more`]). Rows are the caller's to make clickable.

use crate::theme::Theme;
use gpui::{AnyElement, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString, Stateful, Styled, div, prelude::*};

/// A column: its title, its width (`None`: the room left), its cells right-aligned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    pub title: &'static str,
    pub width: Option<f32>,
    pub right: bool,
}

impl Column {
    /// The column that takes the room left (one per table, usually the first).
    pub const fn rest(title: &'static str) -> Self {
        Self { title, width: None, right: false }
    }

    pub const fn fixed(title: &'static str, width: f32) -> Self {
        Self { title, width: Some(width), right: false }
    }

    pub const fn right(mut self) -> Self {
        self.right = true;
        self
    }
}

fn cell(col: &Column, content: AnyElement, t: &Theme) -> gpui::Div {
    let d = div().min_w_0().overflow_hidden().flex().items_center().when(col.right, |d| d.justify_end());
    match col.width {
        Some(w) => d.flex_none().w(t.size(w)),
        None => d.flex_1(),
    }
    .child(content)
}

/// The header row: the columns' titles, small and dim, over a soft rule.
pub fn table_head(cols: &[Column], t: &Theme) -> gpui::Div {
    let mut row = div().h(t.row_h).px_2().flex().items_center().gap_3().border_b_1().border_color(t.line_soft).text_size(t.text_small).text_color(t.dim);
    for c in cols {
        row = row.child(cell(c, div().truncate().child(SharedString::from(c.title)).into_any_element(), t));
    }
    row
}

/// A row: one cell per column, in order (missing ones are empty).
pub fn table_row(id: impl Into<ElementId>, cols: &[Column], cells: Vec<AnyElement>, t: &Theme) -> Stateful<gpui::Div> {
    let mut row = div().id(id).h(t.row_h).px_2().flex().items_center().gap_3().rounded(t.radius_s);
    let mut cells = cells.into_iter();
    for c in cols {
        row = row.child(cell(c, cells.next().unwrap_or_else(|| div().into_any_element()), t));
    }
    row
}

/// What a row holds under it, opened in place: indented, a rule at its left.
pub fn table_more(content: impl IntoElement, t: &Theme) -> gpui::Div {
    div().ml(t.size(20.)).mr_2().mb_2().pl_2().border_l_2().border_color(t.line).flex().flex_col().gap_1().child(content)
}
