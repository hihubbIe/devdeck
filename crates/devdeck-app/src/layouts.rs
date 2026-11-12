//! Named layouts in the UI (R3.10), "pulses" to the user: their menu and the name prompt
//! that makes an empty one, saves the current arrangement under a name or renames one
//! (refusals show in it).

use crate::deck::Deck;
use crate::name_prompt::{self, Submit};
use devdeck_core::layout_book::LayoutId;
use devdeck_ui::MenuItem;
use gpui::{App, Entity, Window};
use std::rc::Rc;

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
    items.push(MenuItem::new("New pulse", move |window, cx| open_new(d.clone(), window, cx)));
    let d = deck.clone();
    items.push(MenuItem::new("Save pulse as", move |window, cx| open_save_as(d.clone(), window, cx)));
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
