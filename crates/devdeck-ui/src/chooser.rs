//! The chooser: what to open and where, in one modal of two columns (the app's "Open in").
//! Each row of the first column (WHAT) has its own list for the second (WHERE). It opens
//! with the first of each selected and the keys in it: ↑↓ move in the column that has
//! them, ←→ and Tab go to the other one; a click selects (its column gets the keys), a
//! double click on a WHERE row chooses, Enter or the confirm button chooses, Esc, Cancel
//! or a press outside close. Both columns are [`CHOOSER_ROWS`] rows high whatever their
//! lists hold, so moving never resizes the modal (R9.10).

use crate::overlay::{ModalClose, ModalNext, ModalPrev, remove_modal};
use crate::scroll::Scrolls;
use crate::theme::{Theme, theme};
use crate::{button, label, modal_frame, push_modal};
use gpui::{
    App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, WeakFocusHandle, Window, actions, div, prelude::*,
};
use std::rc::Rc;

actions!(chooser, [ChooserUp, ChooserDown, ChooserLeft, ChooserRight, ChooserConfirm]);

/// Rows each column shows; a longer list scrolls.
pub const CHOOSER_ROWS: usize = 12;

/// One row: `value` is what a choice hands back.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceRow {
    pub value: String,
    pub label: SharedString,
    pub detail: SharedString,
}

impl ChoiceRow {
    pub fn new(value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Self { value: value.into(), label: label.into(), detail: SharedString::default() }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = detail.into();
        self
    }
}

/// A WHAT row and the WHERE rows it can go to.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub what: ChoiceRow,
    pub wheres: Vec<ChoiceRow>,
}

/// What a choice does: given the WHAT and WHERE values, after the chooser closed.
pub type OnChoose = Rc<dyn Fn(String, String, &mut Window, &mut App)>;

/// What a chooser shows and does.
#[derive(Clone)]
pub struct ChooserSpec {
    pub title: SharedString,
    /// The columns' headings.
    pub what_title: SharedString,
    pub where_title: SharedString,
    pub choices: Vec<Choice>,
    /// The confirm button's label ("Open").
    pub confirm: SharedString,
    /// The WHERE row selected first (by value) when the first WHAT has it.
    pub start_where: Option<String>,
    pub on_choose: OnChoose,
}

/// The column that has the keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    What,
    Where,
}

/// Opens a chooser over the window, the keys in it.
pub fn open_chooser(spec: ChooserSpec, window: &mut Window, cx: &mut App) -> Entity<Chooser> {
    let restore = window.focused(cx).map(|f| f.downgrade());
    let chooser = cx.new(|cx| Chooser::new(spec, restore, cx));
    push_modal(chooser.clone().into(), window, cx);
    let f = chooser.read(cx).focus.clone();
    f.focus(window, cx);
    chooser
}

pub struct Chooser {
    spec: ChooserSpec,
    pub what: usize,
    pub place: usize,
    pub column: Column,
    focus: FocusHandle,
    restore: Option<WeakFocusHandle>,
    /// The columns' scrolls.
    scroll_what: gpui::ScrollHandle,
    scroll_where: gpui::ScrollHandle,
}

impl Chooser {
    fn new(spec: ChooserSpec, restore: Option<WeakFocusHandle>, cx: &mut Context<Self>) -> Self {
        let place = spec.choices.first().and_then(|c| spec.start_where.as_ref().and_then(|v| c.wheres.iter().position(|w| &w.value == v))).unwrap_or(0);
        Self { spec, what: 0, place, column: Column::What, focus: cx.focus_handle(), restore, scroll_what: gpui::ScrollHandle::new(), scroll_where: gpui::ScrollHandle::new() }
    }

    fn wheres(&self) -> &[ChoiceRow] {
        self.spec.choices.get(self.what).map_or(&[], |c| c.wheres.as_slice())
    }

    /// The WHAT and WHERE values selected, when both lists have a row.
    pub fn selected(&self) -> Option<(String, String)> {
        let c = self.spec.choices.get(self.what)?;
        Some((c.what.value.clone(), c.wheres.get(self.place)?.value.clone()))
    }

    /// Selects WHAT row `i`; its WHERE list keeps the row of the same value when it has
    /// one, else its first.
    pub fn set_what(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(c) = self.spec.choices.get(i) else { return };
        let kept = self.wheres().get(self.place).map(|w| w.value.clone());
        self.place = kept.and_then(|v| c.wheres.iter().position(|w| w.value == v)).unwrap_or(0);
        self.what = i;
        self.scroll_what.scroll_to_item(i);
        self.scroll_where.scroll_to_item(self.place);
        cx.notify();
    }

    pub fn set_place(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < self.wheres().len() {
            self.place = i;
            self.scroll_where.scroll_to_item(i);
            cx.notify();
        }
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let (at, n) = match self.column {
            Column::What => (self.what, self.spec.choices.len()),
            Column::Where => (self.place, self.wheres().len()),
        };
        if n == 0 {
            return;
        }
        let to = (at as isize + by).clamp(0, n as isize - 1) as usize;
        match self.column {
            Column::What => self.set_what(to, cx),
            Column::Where => self.set_place(to, cx),
        }
    }

    fn set_column(&mut self, column: Column, cx: &mut Context<Self>) {
        self.column = column;
        cx.notify();
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((what, place)) = self.selected() else { return };
        let on_choose = self.spec.on_choose.clone();
        self.close(window, cx);
        on_choose(what, place, window, cx);
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Its own modal, whatever was pushed over it.
        remove_modal(cx.entity_id(), cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    /// One column: its heading and its rows, `CHOOSER_ROWS` high.
    fn column(&self, which: Column, rows: &[ChoiceRow], selected: usize, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let (id, heading, scroll) = match which {
            Column::What => ("chooser-what", self.spec.what_title.clone(), &self.scroll_what),
            Column::Where => ("chooser-where", self.spec.where_title.clone(), &self.scroll_where),
        };
        let keys = self.column == which;
        let mut list = div().id(id).h(t.row_h * CHOOSER_ROWS as f32).scroll_y().track_scroll(scroll).flex().flex_col();
        for (i, row) in rows.iter().enumerate() {
            let on = i == selected;
            let name = format!("{id}-{i}");
            list = list.child(
                div()
                    .id((id, i))
                    .debug_selector(move || name.clone())
                    .h(t.row_h)
                    .flex_none()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(t.radius_s)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    // The selection shows in both, brighter where the keys are.
                    .when(on, |d| d.bg(if keys { t.selection } else { t.line }))
                    .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        this.column = which;
                        match which {
                            Column::What => this.set_what(i, cx),
                            Column::Where => this.set_place(i, cx),
                        }
                        cx.notify();
                        if which == Column::Where && e.click_count() == 2 {
                            this.confirm(window, cx);
                        }
                    }))
                    .child(label(row.label.clone()).flex_1().text_color(t.fg))
                    .child(div().flex_none().max_w(t.size(160.)).truncate().text_color(t.mute).child(row.detail.clone())),
            );
        }
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap_1()
            .child(div().px_2().text_size(t.text_small).text_color(t.dim).child(heading))
            .child(crate::with_scrollbar((id, usize::MAX), list, scroll))
    }
}

impl Focusable for Chooser {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Chooser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let whats: Vec<ChoiceRow> = self.spec.choices.iter().map(|c| c.what.clone()).collect();
        let wheres = self.wheres().to_vec();
        let what = self.column(Column::What, &whats, self.what, &t, cx).w(t.size(220.)).flex_none();
        let place = self.column(Column::Where, &wheres, self.place, &t, cx).flex_1();
        let content = div()
            .key_context("Modal Chooser")
            .track_focus(&self.focus)
            .w(t.size(620.))
            .max_w(window.viewport_size().width - t.size(80.))
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &ChooserUp, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &ChooserDown, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &ChooserLeft, _, cx| this.set_column(Column::What, cx)))
            .on_action(cx.listener(|this, _: &ChooserRight, _, cx| this.set_column(Column::Where, cx)))
            .on_action(cx.listener(|this, _: &ModalNext, _, cx| {
                let other = if this.column == Column::What { Column::Where } else { Column::What };
                this.set_column(other, cx)
            }))
            .on_action(cx.listener(|this, _: &ModalPrev, _, cx| {
                let other = if this.column == Column::What { Column::Where } else { Column::What };
                this.set_column(other, cx)
            }))
            .on_action(cx.listener(|this, _: &ChooserConfirm, window, cx| this.confirm(window, cx)))
            .child(div().text_color(t.hi).truncate().child(self.spec.title.clone()))
            .child(div().flex().gap_3().min_w_0().child(what).child(place))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(button("chooser-cancel", "Cancel", &t).debug_selector(|| "chooser-cancel".into()).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))))
                    .child(
                        button("chooser-confirm", self.spec.confirm.clone(), &t)
                            .debug_selector(|| "chooser-confirm".into())
                            .on_click(cx.listener(|this, _, w, cx| this.confirm(w, cx))),
                    ),
            );
        modal_frame(content.into_any_element(), window, &t)
    }
}
