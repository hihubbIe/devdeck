//! Named layouts in the UI (R3.10): the `layout ▾` menu, and the modal that saves the
//! current arrangement under a name or renames one. Refusals (empty name, name taken)
//! are shown in the modal; Esc cancels.

use crate::deck::Deck;
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{InputEvent, MenuItem, ModalFocus, TextInput, label, modal_frame, pop_modal, push_modal, theme};
use gpui::{
    App, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    WeakFocusHandle, Window, div,
};

/// The `layout ▾` menu: every layout (current checked), then save as.
pub fn menu(deck: &Entity<Deck>, cx: &App) -> Vec<MenuItem> {
    let book = &deck.read(cx).book;
    let mut items: Vec<MenuItem> = book
        .names()
        .into_iter()
        .map(|name| {
            let (d, n) = (deck.clone(), name.clone());
            MenuItem::new(name.clone(), move |_, cx| {
                d.update(cx, |d, cx| {
                    let _ = d.switch_layout(&n, cx);
                })
            })
            .checked(name == book.current)
        })
        .collect();
    items.push(MenuItem::separator());
    let d = deck.clone();
    items.push(MenuItem::new("Save layout as…", move |window, cx| open_save_as(d.clone(), window, cx)));
    items
}

pub fn open_save_as(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    open(deck, None, window, cx);
}

pub fn open_rename(deck: Entity<Deck>, name: String, window: &mut Window, cx: &mut App) {
    open(deck, Some(name), window, cx);
}

fn open(deck: Entity<Deck>, rename: Option<String>, window: &mut Window, cx: &mut App) {
    let restore = window.focused(cx).map(|f| f.downgrade());
    let modal = cx.new(|cx| NameLayout::new(deck, rename, restore, window, cx));
    push_modal(modal.clone().into(), window, cx);
    let f = modal.read(cx).input.read(cx).focus_handle(cx);
    f.focus(window, cx);
}

pub struct NameLayout {
    deck: Entity<Deck>,
    /// `Some(old name)` when renaming.
    rename: Option<String>,
    pub input: Entity<TextInput>,
    pub error: Option<SharedString>,
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
    _sub: Subscription,
}

impl NameLayout {
    fn new(deck: Entity<Deck>, rename: Option<String>, restore: Option<WeakFocusHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut i = TextInput::new("layout name", cx);
            if let Some(n) = &rename {
                i.set_text(n, cx);
            }
            i
        });
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::Submit(_) => this.submit(window, cx),
            InputEvent::Changed(_) => {
                this.error = None;
                cx.notify();
            }
            InputEvent::Cancel => {}
        });
        let focus = ModalFocus(vec![input.read(cx).focus_handle(cx)]);
        Self { deck, rename, input, error: None, focus, restore, _sub: sub }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).text().to_string();
        let result = match self.rename.clone() {
            None => self.deck.update(cx, |d, cx| d.save_layout_as(&name, cx)),
            Some(old) => self.deck.update(cx, |d, cx| d.rename_layout(&old, &name, cx)),
        };
        match result {
            Ok(()) => self.close(window, cx),
            Err(e) => {
                self.error = Some(e.to_string().into());
                cx.notify();
            }
        }
    }
}

impl Render for NameLayout {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let title = match &self.rename {
            None => "Save layout as".to_string(),
            Some(n) => format!("Rename layout {n}"),
        };
        let content = div()
            .key_context("Modal")
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .child(div().text_color(t.hi).child(title))
            .child(div().w(t.size(360.)).debug_selector(|| "layout-name".into()).child(self.input.clone()))
            .children(self.error.clone().map(|e| label(e).text_color(t.err)));
        modal_frame(content.into_any_element(), window, &t)
    }
}
