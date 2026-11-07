//! Cheat sheet (R5.4, `?` or ctrl+/): every action of the registry by group with its
//! keys, filtered as you type. Esc closes.

use crate::registry::{self, ActionSpec};
use devdeck_core::fuzzy;
use devdeck_ui::overlay::ModalClose;
use devdeck_ui::{InputEvent, TextInput, label, modal_frame, pop_modal, push_modal, section, theme};
use gpui::{
    App, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, WeakFocusHandle, Window, div,
};

pub fn open(window: &mut Window, cx: &mut App) {
    let restore = window.focused(cx).map(|f| f.downgrade());
    let sheet = cx.new(|cx| CheatSheet::new(restore, window, cx));
    push_modal(sheet.clone().into(), window, cx);
    let f = sheet.read(cx).filter.read(cx).focus_handle(cx);
    f.focus(window, cx);
}

pub struct CheatSheet {
    pub filter: Entity<TextInput>,
    restore: Option<WeakFocusHandle>,
    _sub: Subscription,
}

impl CheatSheet {
    fn new(restore: Option<WeakFocusHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| TextInput::new("filter keys and actions", cx));
        let sub = cx.subscribe_in(&filter, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::Cancel => this.close(window, cx),
            _ => cx.notify(),
        });
        Self { filter, restore, _sub: sub }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    /// Entries matching the filter, grouped in registry order, one per label and group.
    pub fn entries(&self, cx: &App) -> Vec<&'static ActionSpec> {
        let q = self.filter.read(cx).text().to_string();
        let mut seen = std::collections::BTreeSet::new();
        registry::all()
            .iter()
            .filter(|s| !s.keys.is_empty() && seen.insert((s.group, s.label)))
            .filter(|s| fuzzy::score(&q, &format!("{} {} {}", s.label, s.group, s.key_text().unwrap_or_default())).is_some())
            .collect()
    }
}

impl Render for CheatSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let entries = self.entries(cx);
        let mut list = div().id("cheat-list").max_h(t.size(560.)).w(t.size(640.)).overflow_y_scroll().flex().flex_col();
        let mut group = "";
        for s in &entries {
            if s.group != group {
                group = s.group;
                list = list.child(section(group, &t));
            }
            let keys = s.keys.iter().map(|k| registry::key_text(k)).collect::<Vec<_>>().join(" / ");
            list = list.child(
                div()
                    .h(t.row_h)
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_none().w(t.size(200.)).truncate().text_color(t.fg).child(keys))
                    .child(label(s.label).text_color(t.mute)),
            );
        }
        if entries.is_empty() {
            list = list.child(label("no match").text_color(t.dim));
        }
        let content = div()
            .key_context("Modal CheatSheet")
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .child(div().text_color(t.hi).child("Keys"))
            .child(div().debug_selector(|| "cheat-filter".into()).child(self.filter.clone()))
            .child(list)
            .child(label("type to filter · esc closes").text_color(t.dim));
        modal_frame(content.into_any_element(), window, &t)
    }
}
