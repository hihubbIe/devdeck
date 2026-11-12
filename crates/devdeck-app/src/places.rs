//! Where something opens (R6.13): a pane of the workspace, named as "Send to" lists
//! them ("n · window · active tab"), or a new window. "Open in" (`crate::open_in`) offers
//! them inside a pulse.

use crate::deck::Deck;
use devdeck_core::layout::{Drop, Layout, MAIN_WINDOW, PaneId, Result, ViewRef, WindowId};
use gpui::{App, Entity};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Pane(PaneId),
    NewWindow,
}

/// Every pane in its numbered order, with where it is and its active tab (named by
/// `title`): "main window · run web", "detached window 2 · empty".
pub fn panes(l: &Layout, title: impl Fn(&ViewRef) -> String) -> Vec<(PaneId, String)> {
    let numbered = l.numbered_panes();
    let mut detached: Vec<_> = numbered.iter().map(|(_, w)| *w).filter(|w| *w != MAIN_WINDOW).collect();
    detached.dedup();
    numbered
        .iter()
        .map(|(pane, win)| {
            let active = l.pane(*pane).ok().and_then(|p| p.active_view().cloned()).map(|r| title(&r)).unwrap_or_else(|| "empty".into());
            let place = if *win == MAIN_WINDOW {
                "main window".to_string()
            } else {
                let nth = detached.iter().position(|w| w == win).map_or(0, |i| i + 1);
                format!("detached window {nth}")
            };
            (*pane, format!("{place} · {active}"))
        })
        .collect()
}

/// Opens `view` at `place`; an open one moves there. Returns the pane showing it.
pub fn open(l: &mut Layout, view: ViewRef, place: Place) -> Result<PaneId> {
    match place {
        Place::Pane(p) if l.find_view(&view).is_some_and(|(q, _)| q != p) => {
            crate::sidebar::open_at(l, view.clone(), Drop::Into(p))?;
            Ok(p)
        }
        Place::Pane(p) => l.open(view, p),
        Place::NewWindow => {
            let (p, i) = match l.find_view(&view) {
                Some(at) => at,
                None => {
                    let p = l.open(view.clone(), l.main().focused)?;
                    (p, l.pane(p)?.tabs.len() - 1)
                }
            };
            l.detach(p, i)?;
            Ok(l.find_view(&view).expect("just detached").0)
        }
    }
}

/// Where `view` shows: the pane holding it, else the main window's focused pane (where
/// it would open).
pub fn showing(l: &Layout, view: &ViewRef) -> PaneId {
    l.find_view(view).map_or(l.main().focused, |(p, _)| p)
}

/// Brings `view` to the front: its tab active where it shows (or opened in the main
/// window's focused pane), a maximized other pane restored so it's drawn, its window
/// raised and keys going to it. `here` is the window the request came from.
pub fn show(deck: &Entity<Deck>, view: &ViewRef, here: WindowId, window: &mut gpui::Window, cx: &mut App) {
    let pane = showing(deck.read(cx).layout(), view);
    let v = view.clone();
    let shown = deck.update(cx, |d, cx| {
        d.apply(cx, |l| {
            let p = open(l, v, Place::Pane(pane))?;
            let w = l.window_of(p)?;
            if let Some(m) = l.window(w)?.maximized.filter(|m| *m != p) {
                l.toggle_maximize(m)?;
            }
            Ok(p)
        })
    });
    let Some(pane) = shown else { return };
    let Ok(w) = deck.read(cx).layout().window_of(pane) else { return };
    if w == here {
        if let Some(f) = deck.read(cx).view_focus(pane) {
            f.focus(window, cx);
        }
        return;
    }
    // Another window: raised, then the view focused in it (once the menu's window let go).
    let handle = match w {
        MAIN_WINDOW => crate::shell::try_shell(cx).and_then(|s| s.read(cx).main_window()),
        _ => deck.read(cx).os_window(w),
    };
    let Some(handle) = handle else { return };
    let deck = deck.clone();
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            window.activate_window();
            if let Some(f) = deck.read(cx).view_focus(pane) {
                f.focus(window, cx);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(s: &str) -> ViewRef {
        ViewRef::new(s)
    }

    #[test]
    fn opening_in_a_pane_or_a_new_window() {
        let mut l = Layout::new();
        let a = l.main().focused;
        l.open(view("app:web"), a).unwrap();
        l.open(view("app:api"), a).unwrap();
        l.move_tab(a, 1, Drop::Edge(a, devdeck_core::layout::Edge::Right)).unwrap();
        let b = l.find_view(&view("app:api")).unwrap().0;
        assert_eq!(open(&mut l, view("term:1"), Place::Pane(b)).unwrap(), b);
        assert_eq!(l.find_view(&view("term:1")).map(|(p, _)| p), Some(b));
        // An open one moves.
        assert_eq!(open(&mut l, view("term:1"), Place::Pane(a)).unwrap(), a);
        assert_eq!(l.find_view(&view("term:1")).map(|(p, _)| p), Some(a));
        let p = open(&mut l, view("term:2"), Place::NewWindow).unwrap();
        assert!(l.window_of(p).is_ok_and(|w| l.is_detached(w)));
        assert_eq!(l.pane(p).unwrap().tabs, [view("term:2")]);
        // An open one goes to a window of its own too.
        let q = open(&mut l, view("term:1"), Place::NewWindow).unwrap();
        assert!(l.window_of(q).is_ok_and(|w| l.is_detached(w)) && q != p);
        assert_eq!(l.pane(q).unwrap().tabs, [view("term:1")]);
        assert_eq!(l.find_view(&view("app:web")).map(|(p, _)| p), Some(a), "what was beside it stayed");
    }
}
