//! The right-click menu (R5.7): a view gives its own items and what was right-clicked
//! ([`Content`]); every action that can take that content adds itself, so an action
//! shows only where it applies. Content so far: text selected in a terminal, a file or a
//! note (source or rendered). Actions: Copy, and Share with, which pastes the text into
//! another terminal of the workspace (Claude Code running there, for one), as it is.

use crate::deck::{Deck, TerminalTabs};
use crate::places;
use crate::runner::{RunKey, Runner};
use crate::services::Services;
use devdeck_core::layout::{ViewRef, WindowId};
use devdeck_core::terminal::paste_bytes;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{MenuItem, open_menu};
use gpui::{App, ClipboardItem, Entity, EntityId, Global, Pixels, Point, WeakEntity, Window};
use std::collections::{BTreeMap, HashMap};

/// What a right click landed on, that actions can take.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// Selected text.
    Text(String),
}

/// Where the menu opens: the view's workspace (its deck and services), its window, and
/// the terminal the content comes from (never offered as where it goes).
#[derive(Clone)]
pub struct Origin {
    pub deck: WeakEntity<Deck>,
    pub services: Entity<Services>,
    pub window: WindowId,
    pub source: Option<RunKey>,
}

/// An action: the items it offers for some content (none when it can't take it).
type Action = fn(&Content, &Origin, &App) -> Vec<MenuItem>;

const ACTIONS: [Action; 2] = [copy, share_with];

/// The view's own items, then every action's for `content`.
pub fn items(own: Vec<MenuItem>, content: Option<&Content>, origin: &Origin, cx: &App) -> Vec<MenuItem> {
    let offered: Vec<MenuItem> = content.map(|c| ACTIONS.iter().flat_map(|a| a(c, origin, cx)).collect()).unwrap_or_default();
    let mut all = own;
    if !all.is_empty() && !offered.is_empty() {
        all.push(MenuItem::separator());
    }
    all.extend(offered);
    all
}

/// The menu of a view showing text in gpui-component's fields (a file, a note): their
/// own items, then what takes the selection. `ctx` is the view's.
pub fn text_menu(ctx: &crate::views::ViewCx) -> devdeck_ui::MenuOpener {
    let ctx = ctx.clone();
    std::rc::Rc::new(move |at, selected, own, window, cx| {
        let Some(deck) = ctx.deck.upgrade() else { return };
        let l = deck.read(cx).layout();
        let Some(here) = l.find_view(&ctx.view).and_then(|(p, _)| l.window_of(p).ok()) else { return };
        let origin = Origin { deck: ctx.deck.clone(), services: ctx.services.clone(), window: here, source: None };
        open(at, own, selected.map(Content::Text), &origin, window, cx);
    })
}

/// Opens the menu at `at`; nothing opens when there's nothing to offer. Returns whether
/// it opened.
pub fn open(at: Point<Pixels>, own: Vec<MenuItem>, content: Option<Content>, origin: &Origin, window: &mut Window, cx: &mut App) -> bool {
    let items = items(own, content.as_ref(), origin, cx);
    if items.is_empty() {
        return false;
    }
    open_menu(at, items, window, cx);
    true
}

fn copy(content: &Content, _: &Origin, _: &App) -> Vec<MenuItem> {
    let Content::Text(text) = content;
    let text = text.clone();
    vec![MenuItem::new("Copy", move |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())))]
}

/// The last terminal each workspace's text went to (by its runner): Share with's own
/// click goes there again.
#[derive(Default)]
struct LastShared(HashMap<EntityId, RunKey>);

impl Global for LastShared {}

/// A terminal text can go to: its key, name, the view showing it, and why it can't take
/// text if it can't.
struct Target {
    key: RunKey,
    name: String,
    view: ViewRef,
    refused: Option<&'static str>,
}

/// The workspace's running terminals but `source`, by number, named by their folder (with
/// their number when two share one). Only a terminal whose program takes pastes as such
/// (bracketed paste) gets text: elsewhere each line would run as a command.
fn targets(runner: &Runner, tabs: &BTreeMap<u32, TerminalTabs>, source: Option<&RunKey>) -> Vec<Target> {
    let mut found: Vec<(u32, RunKey, String)> = runner
        .terminals()
        .into_iter()
        .filter(|(k, _)| Some(k) != source && runner.state(k).is_some_and(|s| s.is_running()))
        .filter_map(|(k, dir)| Some((k.terminal_number()?, k, dir.display().to_string())))
        .collect();
    found.sort_by_key(|(n, _, _)| *n);
    let names: Vec<String> = found.iter().map(|(_, _, dir)| crate::views::terminal_name(dir)).collect();
    found
        .into_iter()
        .zip(names.iter())
        .map(|((n, key, dir), name)| {
            let twice = names.iter().filter(|m| *m == name).count() > 1;
            // The tab a layout shows it by (so showing it never adds a second), else a new one.
            let view = tabs.get(&n).map(|t| t.view.clone()).unwrap_or_else(|| ViewAddr::Terminal { n, dir }.to_ref());
            let pastes = runner.run(&key).is_some_and(|r| r.terminal.modes().bracketed_paste);
            Target { key, name: if twice { format!("{name} · {n}") } else { name.clone() }, view, refused: (!pastes).then_some(NO_SAFE_PASTE) }
        })
        .collect()
}

/// Text without control characters but line breaks and tabs: output can't press keys in
/// the terminal it goes to.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect()
}

/// Pastes `text` into terminal `to` as one block, control characters removed, no Enter
/// (the question is still to type), and remembers it as where text went last. Needs no
/// window: the UI and the control API share this. Refused unless the terminal takes
/// pastes safely (bracketed paste).
pub fn paste(runner: &Entity<Runner>, to: &RunKey, text: &str, cx: &mut App) -> Result<(), &'static str> {
    if !runner.read(cx).run(to).is_some_and(|r| r.terminal.modes().bracketed_paste) {
        return Err(NO_SAFE_PASTE);
    }
    let bytes = paste_bytes(&clean(text), true);
    runner.update(cx, |r, _| r.write(to, &bytes));
    cx.default_global::<LastShared>().0.insert(runner.entity_id(), to.clone());
    Ok(())
}

/// Why a terminal can't take a paste.
const NO_SAFE_PASTE: &str = "doesn't take pastes safely";

/// [`paste`], then shows the terminal, keys going to it.
pub fn share(text: &str, to: &RunKey, view: &ViewRef, origin: &Origin, window: &mut Window, cx: &mut App) {
    let runner = origin.services.read(cx).runner.clone();
    if paste(&runner, to, text, cx).is_err() {
        return;
    }
    if let Some(deck) = origin.deck.upgrade() {
        places::show(&deck, view, origin.window, window, cx);
    }
}

fn share_with(content: &Content, origin: &Origin, cx: &App) -> Vec<MenuItem> {
    let Content::Text(text) = content;
    let runner = origin.services.read(cx).runner.clone();
    let tabs = origin.deck.upgrade().map(|d| d.read(cx).terminals(cx)).unwrap_or_default();
    let targets = targets(runner.read(cx), &tabs, origin.source.as_ref());
    let send = |t: &Target| {
        let (text, key, view, origin) = (text.clone(), t.key.clone(), t.view.clone(), origin.clone());
        move |window: &mut Window, cx: &mut App| share(&text, &key, &view, &origin, window, cx)
    };
    let sub: Vec<MenuItem> = targets
        .iter()
        .map(|t| {
            let item = MenuItem::new(t.name.clone(), send(t));
            match t.refused {
                Some(why) => item.disabled(why),
                None => item,
            }
        })
        .collect();
    // Its own click: where text went last, else the first terminal that takes it.
    let last = cx.try_global::<LastShared>().and_then(|l| l.0.get(&runner.entity_id()));
    let usable = |t: &&Target| t.refused.is_none();
    let default = targets.iter().filter(usable).find(|t| Some(&t.key) == last).or_else(|| targets.iter().find(usable));
    let item = match default {
        Some(t) => MenuItem::new("Share with", send(t)),
        None if targets.is_empty() => MenuItem::new("Share with", |_, _| {}).disabled("no other terminal open"),
        // Kept open so its list says why each terminal can't take it.
        None => MenuItem::new("Share with", |_, _| {}),
    };
    vec![item.submenu(sub)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_never_reach_the_terminal_but_lines_do() {
        assert_eq!(clean("a\x03b\x1b[31mc\u{9b}d\n\te"), "ab[31mcd\n\te");
    }
}
