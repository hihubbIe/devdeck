//! `Deck` owns the layout model, the view instances and the OS windows.
//!
//! Every layout change goes through [`Deck::apply`], which then
//! - creates views for newly opened tabs and drops views whose tab closed,
//! - opens or closes OS windows so they match the model's windows,
//! - notifies observers so every window re-renders.

use crate::views;
use crate::window_view::WindowView;
use devdeck_core::layout::{CloseOutcome, Layout, LayoutError, MAIN_WINDOW, ViewRef, WindowId};
use gpui::{
    AnyView, AnyWeakView, App, AppContext, Bounds, Context, Entity, WindowBounds, WindowDecorations,
    WindowHandle, WindowOptions, px, size,
};
use std::collections::HashMap;

pub struct Deck {
    pub layout: Layout,
    views: HashMap<ViewRef, AnyView>,
    /// `None` while the OS window is being opened (deferred).
    os_windows: HashMap<WindowId, Option<WindowHandle<WindowView>>>,
    /// Views closed recently, checked for release by the leak test.
    released: Vec<(AnyWeakView, u64)>,
    pub last_error: Option<String>,
    generation: u64,
}

impl Deck {
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            views: HashMap::new(),
            os_windows: HashMap::new(),
            released: Vec::new(),
            last_error: None,
            generation: 0,
        }
    }

    pub fn view(&self, r: &ViewRef) -> Option<AnyView> {
        self.views.get(r).cloned()
    }

    pub fn live_views(&self) -> usize {
        self.views.len()
    }

    /// Runs a layout operation and brings views and windows in line with the result.
    pub fn apply<T>(&mut self, cx: &mut Context<Self>, op: impl FnOnce(&mut Layout) -> Result<T, LayoutError>) -> Option<T> {
        let out = match op(&mut self.layout) {
            Ok(v) => {
                self.last_error = None;
                Some(v)
            }
            Err(e) => {
                self.last_error = Some(describe(&e));
                None
            }
        };
        debug_assert!(self.layout.validate().is_ok(), "{:?}", self.layout.validate());
        self.sync(cx);
        cx.notify();
        out
    }

    /// Closing the main window quits; closing a detached one re-attaches its tabs.
    pub fn close_window(&mut self, id: WindowId, cx: &mut Context<Self>) {
        if let Some(CloseOutcome::Quit) = self.apply(cx, |l| l.close_window(id)) {
            cx.quit();
        }
    }

    /// Called when the OS closed a window on its own (window manager close).
    pub fn os_window_closed(&mut self, id: WindowId, cx: &mut Context<Self>) {
        self.os_windows.remove(&id);
        if id == MAIN_WINDOW {
            cx.quit();
        } else if self.layout.window(id).is_ok() {
            self.close_window(id, cx);
        }
    }

    pub fn sync(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        // Views: create missing, drop closed.
        let open = self.layout.views();
        let closed: Vec<ViewRef> = self.views.keys().filter(|r| !open.contains(*r)).cloned().collect();
        for r in closed {
            if let Some(v) = self.views.remove(&r) {
                self.released.push((v.downgrade(), self.generation));
            }
        }
        for r in open {
            if let std::collections::hash_map::Entry::Vacant(slot) = self.views.entry(r) {
                let v = views::build(slot.key(), cx);
                slot.insert(v);
            }
        }
        // Windows: open missing OS windows, close the ones the model dropped.
        let model: Vec<WindowId> = self.layout.windows.iter().map(|w| w.id).collect();
        let gone: Vec<WindowId> = self.os_windows.keys().filter(|id| !model.contains(id)).copied().collect();
        for id in gone {
            if let Some(Some(h)) = self.os_windows.remove(&id) {
                cx.defer(move |cx| {
                    let _ = h.update(cx, |_, window, _| window.remove_window());
                });
            }
        }
        let missing: Vec<WindowId> = model.into_iter().filter(|id| !self.os_windows.contains_key(id)).collect();
        for id in missing {
            // Opening a window renders it at once, and rendering reads the deck, so
            // open it after this update finishes.
            self.os_windows.insert(id, None);
            let deck = cx.entity();
            cx.defer(move |cx| open_os_window(deck, id, cx));
        }
    }

    /// Views closed more than `age` generations ago that are still alive.
    pub fn leaked_views(&mut self, age: u64) -> usize {
        let now = self.generation;
        let mut leaked = 0;
        self.released.retain(|(weak, at)| {
            if now.saturating_sub(*at) < age {
                return true;
            }
            if weak.upgrade().is_some() {
                leaked += 1;
            }
            false
        });
        leaked
    }
}

fn open_os_window(deck: Entity<Deck>, id: WindowId, cx: &mut App) {
    if deck.read(cx).layout.window(id).is_err() {
        deck.update(cx, |d, _| d.os_windows.remove(&id));
        return;
    }
    let is_main = id == MAIN_WINDOW;
    let bounds = Bounds::centered(None, if is_main { size(px(1600.), px(1000.)) } else { size(px(900.), px(600.)) }, cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("devdeck".into()),
        window_min_size: Some(size(px(480.), px(320.))),
        ..Default::default()
    };
    let view_deck = deck.clone();
    let result = cx.open_window(options, |window, cx| cx.new(|cx| WindowView::new(id, view_deck, window, cx)));
    deck.update(cx, |d, cx| match result {
        Ok(h) => {
            d.os_windows.insert(id, Some(h));
        }
        Err(e) => {
            d.os_windows.remove(&id);
            d.last_error = Some(format!("could not open a window: {e}"));
            cx.notify();
        }
    });
}

fn describe(e: &LayoutError) -> String {
    match e {
        LayoutError::MainWindowLastPane => "The main window keeps at least one pane.".into(),
        LayoutError::DetachedWindowIsSinglePane => "A detached window holds one pane; drop into its tabs instead.".into(),
        LayoutError::LastExpandedPane => "Keep at least one pane expanded.".into(),
        LayoutError::AlreadyAlone => "This tab already has its own window.".into(),
        LayoutError::NotDetached => "Only detached windows can be re-attached.".into(),
        other => format!("{other}"),
    }
}
