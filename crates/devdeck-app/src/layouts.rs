//! Named layouts in the UI (R3.10), "pulses" to the user: their menu, "open in pulse",
//! and the name prompt that makes an empty one, saves the current arrangement under a
//! name or renames one (refusals show in it).

use crate::deck::Deck;
use crate::name_prompt::{self, Submit};
use devdeck_core::layout_book::LayoutId;
use devdeck_core::layout::ViewRef;
use devdeck_ui::{MenuItem, ToastKind, toasts};
use gpui::{App, Entity, Window};
use std::rc::Rc;

/// "Open in pulse": every pulse (alphabetical, the current one checked); `view` joins the
/// first pane of the one picked ([`Deck::open_in_pulse`]), and a toast says where.
pub fn open_in_pulse(deck: &Entity<Deck>, view: &ViewRef, cx: &App) -> Vec<MenuItem> {
    let book = &deck.read(cx).book;
    book.entries()
        .into_iter()
        .map(|(id, name)| {
            let current = id == book.current;
            let (d, view) = (deck.clone(), view.clone());
            MenuItem::new(name, move |_, cx| {
                let title = d.read(cx).title(&view, cx);
                let done = d.update(cx, |d, cx| d.open_in_pulse(&id, view.clone(), cx));
                let (kind, text) = match done {
                    Ok(name) => (ToastKind::Info, format!("{title} opened in {name}")),
                    Err(why) => (ToastKind::Warn, format!("{title} wasn't opened: {why}")),
                };
                toasts(cx).update(cx, |t, cx| t.push(kind, text, vec![], cx));
            })
            .checked(current)
        })
        .collect()
}

/// The pulses' menu: every pulse (current checked), then new and save as.
pub fn menu(deck: &Entity<Deck>, cx: &App) -> Vec<MenuItem> {
    let book = &deck.read(cx).book;
    let mut items: Vec<MenuItem> = book
        .entries()
        .into_iter()
        .map(|(id, name)| {
            let current = id == book.current;
            let d = deck.clone();
            MenuItem::new(name, move |_, cx| {
                d.update(cx, |d, cx| {
                    let _ = d.switch_layout(&id, cx);
                })
            })
            .checked(current)
        })
        .collect();
    items.push(MenuItem::separator());
    let d = deck.clone();
    items.push(MenuItem::new("New pulse…", move |window, cx| open_new(d.clone(), window, cx)));
    let d = deck.clone();
    items.push(MenuItem::new("Save pulse as…", move |window, cx| open_save_as(d.clone(), window, cx)));
    items
}

/// Makes an empty layout and shows it.
pub fn open_new(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let submit: Submit = Rc::new(move |name, cx| deck.update(cx, |d, cx| d.new_layout(name, cx).map(drop).map_err(|e| e.to_string())));
    name_prompt::open("New pulse", "", "layout-name", submit, window, cx);
}

pub fn open_save_as(deck: Entity<Deck>, window: &mut Window, cx: &mut App) {
    let submit: Submit = Rc::new(move |name, cx| deck.update(cx, |d, cx| d.save_layout_as(name, cx).map(drop).map_err(|e| e.to_string())));
    name_prompt::open("Save pulse as", "", "layout-name", submit, window, cx);
}

/// Renames layout `id`; the prompt starts on its current name.
pub fn open_rename(deck: Entity<Deck>, id: LayoutId, window: &mut Window, cx: &mut App) {
    let Some(name) = deck.read(cx).book.name(&id).map(str::to_string) else { return };
    let submit: Submit = Rc::new(move |to, cx| deck.update(cx, |d, cx| d.rename_layout(&id, to, cx).map_err(|e| e.to_string())));
    name_prompt::open(format!("Rename pulse {name}"), &name, "layout-name", submit, window, cx);
}
