//! `Deck` owns one workspace's layout model, its view instances and its detached OS
//! windows. The main OS window belongs to the [`Shell`](crate::shell::Shell), which shows
//! the active workspace's deck in it.
//!
//! Every layout change goes through [`Deck::apply`], which then
//! - creates views for newly opened tabs and drops views whose tab closed,
//! - opens or closes detached OS windows so they match the model (only while the
//!   workspace is active; an inactive one keeps its model and closes its windows),
//! - schedules a save of the layout (one timer, about a second after the first change),
//! - notifies observers so every window re-renders.

use crate::views::{self, ViewCx};
use crate::window_view::{WindowView, window_options};
use devdeck_core::layout::{Layout, LayoutError, MAIN_WINDOW, PaneId, ViewRef, WindowId};
use devdeck_core::state::StateStore;
use devdeck_ui::{ToastKind, toasts};
use gpui::{AnyView, AnyWeakView, App, AppContext, Context, Entity, FocusHandle, Task, WindowHandle};
use std::collections::HashMap;
use std::time::Duration;

/// How long after a change the layout is written.
pub const SAVE_DELAY: Duration = Duration::from_millis(1000);

pub struct Deck {
    pub layout: Layout,
    pub workspace: String,
    active: bool,
    views: HashMap<ViewRef, AnyView>,
    /// Focus handles of views that take keys.
    focus: HashMap<ViewRef, FocusHandle>,
    /// What views get to reach their workspace (`None` in bare decks: demo views only).
    ctx: Option<ViewCx>,
    /// Detached windows only. `None` while the OS window is being opened (deferred).
    os_windows: HashMap<WindowId, Option<WindowHandle<WindowView>>>,
    /// Views closed recently, checked for release by the leak test.
    released: Vec<(AnyWeakView, u64)>,
    pub last_error: Option<String>,
    generation: u64,
    store: Option<StateStore>,
    dirty: bool,
    save_task: Option<Task<()>>,
    save_failed: bool,
    /// Layout writes done (tests check the debounce).
    pub saves: usize,
}

impl Deck {
    /// An active deck that is never saved (tests, snapshots).
    pub fn new(layout: Layout) -> Self {
        Self {
            layout,
            workspace: "default".into(),
            active: true,
            views: HashMap::new(),
            focus: HashMap::new(),
            ctx: None,
            os_windows: HashMap::new(),
            released: Vec::new(),
            last_error: None,
            generation: 0,
            store: None,
            dirty: false,
            save_task: None,
            save_failed: false,
            saves: 0,
        }
    }

    /// Ties the deck to a workspace: where it saves, and whether it starts active.
    pub fn in_workspace(mut self, id: impl Into<String>, store: Option<StateStore>, active: bool) -> Self {
        self.workspace = id.into();
        self.store = store;
        self.active = active;
        self
    }

    pub fn with_ctx(mut self, ctx: ViewCx) -> Self {
        self.ctx = Some(ctx);
        self
    }

    /// Where keys go when the active view of `pane` should have them.
    pub fn view_focus(&self, pane: PaneId) -> Option<FocusHandle> {
        let r = self.layout.pane(pane).ok()?.active_view()?;
        self.focus.get(r).cloned()
    }

    /// Whether a view is on screen: active tab of an expanded pane, workspace shown.
    pub fn is_visible(&self, r: &ViewRef) -> bool {
        self.active
            && self.layout.find_view(r).is_some_and(|(p, i)| {
                self.layout.pane(p).is_ok_and(|pane| pane.active == i && !pane.collapsed)
                    && self.layout.window_of(p).ok().and_then(|w| self.layout.window(w).ok()).is_some_and(|w| w.maximized.is_none_or(|m| m == p))
            })
    }

    pub fn view(&self, r: &ViewRef) -> Option<AnyView> {
        self.views.get(r).cloned()
    }

    pub fn live_views(&self) -> usize {
        self.views.len()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Runs a layout operation and brings views and windows in line with the result.
    pub fn apply<T>(&mut self, cx: &mut Context<Self>, op: impl FnOnce(&mut Layout) -> Result<T, LayoutError>) -> Option<T> {
        let out = match op(&mut self.layout) {
            Ok(v) => {
                self.last_error = None;
                self.changed(cx);
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

    /// Shows or hides the workspace: an inactive deck closes its detached OS windows
    /// (the model keeps them) and reopens them when it becomes active again.
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active != active {
            self.active = active;
            self.sync(cx);
            cx.notify();
        }
    }

    /// Closing a detached window closes its tabs too. The main window is the shell's.
    pub fn close_window(&mut self, id: WindowId, cx: &mut Context<Self>) {
        debug_assert_ne!(id, MAIN_WINDOW, "the shell handles the main window");
        if id != MAIN_WINDOW {
            self.apply(cx, |l| l.close_window(id).map(|_| ()));
        }
    }

    /// Sends a detached window's tabs back to the main window (⇲).
    pub fn reattach(&mut self, id: WindowId, cx: &mut Context<Self>) {
        self.apply(cx, |l| l.reattach(id).map(|_| ()));
    }

    /// Called when the window manager closed a detached window.
    pub fn os_window_closed(&mut self, id: WindowId, cx: &mut Context<Self>) {
        self.os_windows.remove(&id);
        if id != MAIN_WINDOW && self.layout.window(id).is_ok() {
            self.close_window(id, cx);
        }
    }

    /// Remembers a detached window's size (saved with the layout, no re-render).
    pub fn record_size(&mut self, id: WindowId, size: (f32, f32), cx: &mut Context<Self>) {
        let same = self.layout.window(id).is_ok_and(|w| w.size == Some(size));
        if !same && self.layout.set_window_size(id, size).is_ok() {
            self.changed(cx);
        }
    }

    pub fn sync(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        // Views: create missing, drop closed.
        let open = self.layout.views();
        let closed: Vec<ViewRef> = self.views.keys().filter(|r| !open.contains(*r)).cloned().collect();
        for r in closed {
            self.focus.remove(&r);
            if let Some(v) = self.views.remove(&r) {
                self.released.push((v.downgrade(), self.generation));
            }
        }
        for r in open {
            if !self.views.contains_key(&r) {
                let built = views::build(&r, self.ctx.as_ref(), cx);
                if let Some(f) = built.focus {
                    self.focus.insert(r.clone(), f);
                }
                self.views.insert(r, built.view);
            }
        }
        // Detached windows: open the missing ones, close the ones the model dropped (or
        // all of them while the workspace is hidden).
        let wanted: Vec<WindowId> = if self.active {
            self.layout.windows.iter().map(|w| w.id).filter(|id| *id != MAIN_WINDOW).collect()
        } else {
            Vec::new()
        };
        let gone: Vec<WindowId> = self.os_windows.keys().filter(|id| !wanted.contains(id)).copied().collect();
        for id in gone {
            if let Some(Some(h)) = self.os_windows.remove(&id) {
                cx.defer(move |cx| {
                    let _ = h.update(cx, |_, window, _| window.remove_window());
                });
            }
        }
        for id in wanted {
            if let std::collections::hash_map::Entry::Vacant(slot) = self.os_windows.entry(id) {
                // Opening a window renders it at once, and rendering reads the deck, so
                // open it after this update finishes.
                slot.insert(None);
                let deck = cx.entity();
                cx.defer(move |cx| open_os_window(deck, id, cx));
            }
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

    // ------------------------------------------------------------------ saving

    /// Marks the layout changed and starts the save timer if none runs.
    fn changed(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() {
            return;
        }
        self.dirty = true;
        if self.save_task.is_none() {
            self.schedule_save(cx);
        }
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let Ok(Some((store, ws, layout))) = this.update(cx, |d, _| d.take_save()) else { return };
            let result = cx.background_executor().spawn(async move { store.save_layout(&ws, &layout) }).await;
            let _ = this.update(cx, |d, cx| {
                d.save_task = None;
                d.saved(result, cx);
                if d.dirty {
                    d.schedule_save(cx);
                }
            });
        }));
    }

    fn take_save(&mut self) -> Option<(StateStore, String, Layout)> {
        let store = self.store.clone()?;
        self.dirty = false;
        self.saves += 1;
        Some((store, self.workspace.clone(), self.layout.clone()))
    }

    /// A failing save is reported once, until a save works again.
    fn saved(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => self.save_failed = false,
            Err(e) if !self.save_failed => {
                self.save_failed = true;
                let text = format!("Could not save the {} layout: {e}", self.workspace);
                toasts(cx).update(cx, |t, cx| t.push(ToastKind::Err, text, vec![], cx));
            }
            Err(_) => {}
        }
    }

    /// Writes a pending change now (quit).
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        if !self.dirty {
            return;
        }
        self.save_task = None;
        if let Some((store, ws, layout)) = self.take_save() {
            let result = store.save_layout(&ws, &layout);
            self.saved(result, cx);
        }
    }
}

fn open_os_window(deck: Entity<Deck>, id: WindowId, cx: &mut App) {
    let size = {
        let d = deck.read(cx);
        let pending = d.os_windows.get(&id).is_some_and(Option::is_none);
        match d.layout.window(id) {
            Ok(w) if d.active && pending => w.size.unwrap_or((900., 600.)),
            _ => {
                // Closed, hidden, or handled while the open was pending.
                if pending {
                    deck.update(cx, |d, _| d.os_windows.remove(&id));
                }
                return;
            }
        }
    };
    let view_deck = deck.clone();
    let result = cx.open_window(window_options(size, cx), |window, cx| cx.new(|cx| WindowView::detached(id, view_deck, window, cx)));
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
