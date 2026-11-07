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
use devdeck_core::layout_book::{BookError, LayoutBook};
use devdeck_core::state::StateStore;
use devdeck_ui::{ToastKind, toasts};
use gpui::{AnyView, AnyWeakView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Task, WindowHandle};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// How long after a change the layout is written.
pub const SAVE_DELAY: Duration = Duration::from_millis(1000);

pub struct Deck {
    /// Read through [`Deck::layout`]; every change goes through [`Deck::apply`].
    layout: Layout,
    pub workspace: String,
    active: bool,
    views: HashMap<ViewRef, AnyView>,
    /// Focus handles of views that take keys.
    focus: HashMap<ViewRef, FocusHandle>,
    /// The workspace's services, handed to every view (`None` in bare decks: demo views only).
    services: Option<Entity<crate::services::Services>>,
    /// Detached windows only. `None` while the OS window is being opened (deferred).
    os_windows: HashMap<WindowId, Option<WindowHandle<WindowView>>>,
    /// Views closed recently, checked for release by the leak test.
    released: Vec<(AnyWeakView, u64)>,
    /// The last refused or failed change, also shown as a toast (see [`Deck::report`]).
    pub last_error: Option<String>,
    generation: u64,
    store: Option<StateStore>,
    dirty: bool,
    save_task: Option<Task<()>>,
    save_failed: bool,
    /// Layout writes done (tests check the debounce).
    pub saves: usize,
    /// A "Send to…" pick in progress (R3.11).
    pub send: Option<SendSource>,
    /// Named layouts of the workspace (R3.10).
    pub book: LayoutBook,
    /// Views on screen now; changes are announced with [`DeckEvent::Visibility`].
    visible: HashSet<ViewRef>,
    /// The main window shows something else over its panes (Settings).
    covered: bool,
}

pub enum DeckEvent {
    /// Views that came on screen and views that left it, since the last change.
    Visibility { shown: Vec<ViewRef>, hidden: Vec<ViewRef> },
}

impl EventEmitter<DeckEvent> for Deck {}

/// What a "Send to…" pick moves.
#[derive(Clone, Debug, PartialEq)]
pub enum SendSource {
    Tab(ViewRef),
    Pane(PaneId),
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
            services: None,
            os_windows: HashMap::new(),
            released: Vec::new(),
            last_error: None,
            generation: 0,
            store: None,
            dirty: false,
            save_task: None,
            save_failed: false,
            saves: 0,
            send: None,
            book: LayoutBook::default(),
            visible: HashSet::new(),
            covered: false,
        }
    }

    /// Ties the deck to a workspace: where it saves, and whether it starts active.
    pub fn in_workspace(mut self, id: impl Into<String>, store: Option<StateStore>, active: bool) -> Self {
        self.workspace = id.into();
        self.store = store;
        self.active = active;
        self
    }

    pub fn with_services(mut self, services: Entity<crate::services::Services>) -> Self {
        self.services = Some(services);
        self
    }

    /// Where keys go when the active view of `pane` should have them.
    pub fn view_focus(&self, pane: PaneId) -> Option<FocusHandle> {
        let r = self.layout.pane(pane).ok()?.active_view()?;
        self.focus.get(r).cloned()
    }

    /// Whether a view is on screen: the shown workspace, its window drawn and not
    /// covered, the active tab of an expanded pane, not hidden by a maximized one.
    pub fn is_visible(&self, r: &ViewRef) -> bool {
        self.visible.contains(r)
    }

    fn compute_visible(&self) -> HashSet<ViewRef> {
        if !self.active {
            return HashSet::new();
        }
        let l = &self.layout;
        let window_drawn = |w: WindowId| if w == MAIN_WINDOW { !self.covered } else { matches!(self.os_windows.get(&w), Some(Some(_))) };
        l.windows
            .iter()
            .filter(|w| window_drawn(w.id))
            .flat_map(|w| {
                let panes = match w.maximized {
                    Some(m) => vec![m],
                    None => l.panes_in(w.id).unwrap_or_default(),
                };
                panes.into_iter().filter_map(|p| l.pane(p).ok().filter(|p| !p.collapsed).and_then(|p| p.active_view().cloned()))
            })
            .collect()
    }

    /// Recomputes what is on screen; the one place visibility changes are announced.
    fn refresh_visibility(&mut self, cx: &mut Context<Self>) {
        let now = self.compute_visible();
        if now == self.visible {
            return;
        }
        let shown: Vec<ViewRef> = now.difference(&self.visible).cloned().collect();
        let hidden: Vec<ViewRef> = self.visible.difference(&now).cloned().collect();
        self.visible = now;
        cx.emit(DeckEvent::Visibility { shown, hidden });
    }

    /// The main window covers its panes (Settings) or shows them again.
    pub fn set_covered(&mut self, covered: bool, cx: &mut Context<Self>) {
        if self.covered != covered {
            self.covered = covered;
            self.refresh_visibility(cx);
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
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
                // A kept view whose tab closed isn't kept any more.
                let before = self.book.keep.len();
                let layout = &self.layout;
                self.book.keep.retain(|v| layout.find_view(v).is_some());
                if self.book.keep.len() != before {
                    self.save_book(cx);
                }
                Some(v)
            }
            Err(e) => {
                self.report(describe(&e), cx);
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
        self.send = None;
        if id != MAIN_WINDOW {
            self.apply(cx, |l| l.close_window(id).map(|_| ()));
        }
    }

    /// Sends a detached window's tabs back to the main window (⇲).
    pub fn reattach(&mut self, id: WindowId, cx: &mut Context<Self>) {
        self.send = None;
        self.apply(cx, |l| l.reattach(id).map(|_| ()));
    }

    /// Called when the window manager closed a detached window.
    pub fn os_window_closed(&mut self, id: WindowId, cx: &mut Context<Self>) {
        self.os_windows.remove(&id);
        self.refresh_visibility(cx);
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

    pub fn with_book(mut self, book: LayoutBook) -> Self {
        self.book = book;
        self
    }

    // ------------------------------------------------------------------ named layouts

    pub fn save_layout_as(&mut self, name: &str, cx: &mut Context<Self>) -> Result<(), BookError> {
        let live = self.layout.clone();
        self.book.save_as(name, &live)?;
        self.save_book(cx);
        cx.notify();
        Ok(())
    }

    /// Shows another named layout: tabs that aren't in it close (their runs go on),
    /// kept views follow, detached windows follow the layout.
    pub fn switch_layout(&mut self, name: &str, cx: &mut Context<Self>) -> Result<(), BookError> {
        if self.book.current.eq_ignore_ascii_case(name) {
            return Ok(());
        }
        let live = self.layout.clone();
        let target = self.book.switch(name, &live)?;
        self.send = None;
        // Ids repeat across stored layouts: drop every detached OS window so the
        // target's open fresh instead of reusing one with the same id.
        for (_, h) in self.os_windows.drain() {
            if let Some(h) = h {
                cx.defer(move |cx| {
                    let _ = h.update(cx, |_, window, _| window.remove_window());
                });
            }
        }
        self.apply(cx, |l| {
            *l = target;
            Ok(())
        });
        // Both files at once, so they never disagree about which tree is current.
        if self.store.is_some() {
            self.save_task = None;
            if let Some((store, ws, layout)) = self.take_save() {
                let result = store.save_layout(&ws, &layout);
                self.saved(result, cx);
            }
            self.save_book(cx);
        }
        Ok(())
    }

    pub fn rename_layout(&mut self, from: &str, to: &str, cx: &mut Context<Self>) -> Result<(), BookError> {
        self.book.rename(from, to)?;
        self.save_book(cx);
        cx.notify();
        Ok(())
    }

    pub fn delete_layout(&mut self, name: &str, cx: &mut Context<Self>) -> Result<(), BookError> {
        self.book.delete(name)?;
        self.save_book(cx);
        cx.notify();
        Ok(())
    }

    pub fn toggle_keep(&mut self, view: &ViewRef, cx: &mut Context<Self>) -> bool {
        let kept = self.book.toggle_keep(view);
        self.save_book(cx);
        cx.notify();
        kept
    }

    /// The one path for a change the deck refused or couldn't make: a toast, where the
    /// user looks for outcomes, and `last_error` for tests.
    fn report(&mut self, text: String, cx: &mut Context<Self>) {
        toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text.clone(), vec![], cx));
        self.last_error = Some(text);
        cx.notify();
    }

    /// Writes the book now: a small file, changed rarely, and writes stay in order.
    fn save_book(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else { return };
        let result = store.save_book(&self.workspace, &self.book);
        self.saved(result, cx);
    }

    /// Starts a pick: badges appear on every pane of the workspace.
    pub fn start_send(&mut self, source: SendSource, cx: &mut Context<Self>) {
        self.send = Some(source);
        cx.notify();
    }

    pub fn cancel_send(&mut self, cx: &mut Context<Self>) {
        if self.send.take().is_some() {
            cx.notify();
        }
    }

    /// Completes a pick: into a pane, or a new window when `None`.
    pub fn send_to(&mut self, target: Option<PaneId>, cx: &mut Context<Self>) {
        let Some(source) = self.send.take() else { return };
        match source {
            SendSource::Tab(view) => self.apply(cx, |l| l.send_tab(&view, target)),
            SendSource::Pane(pane) => self.apply(cx, |l| l.send_pane(pane, target)),
        };
    }

    /// Why a target can't take the pick, if it can't.
    pub fn send_refusal(&self, target: Option<PaneId>) -> Option<&'static str> {
        let source_pane = match self.send.as_ref()? {
            SendSource::Tab(v) => self.layout.find_view(v)?.0,
            SendSource::Pane(p) => *p,
        };
        match target {
            Some(t) if t == source_pane => Some("already here"),
            None => {
                let w = self.layout.window_of(source_pane).ok()?;
                let alone = self.layout.is_detached(w) && self.layout.panes_in(w).is_ok_and(|p| p.len() == 1);
                let single_tab = matches!(self.send, Some(SendSource::Tab(_))) && self.layout.pane(source_pane).is_ok_and(|p| p.tabs.len() == 1);
                (alone && (single_tab || matches!(self.send, Some(SendSource::Pane(_))))).then_some("already alone in its window")
            }
            _ => None,
        }
    }

    pub fn sync(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        // A pick whose source went away is over.
        let gone = match &self.send {
            Some(SendSource::Tab(v)) => self.layout.find_view(v).is_none(),
            Some(SendSource::Pane(p)) => self.layout.pane(*p).is_err(),
            None => false,
        };
        if gone || !self.active {
            self.send = None;
        }
        // Views: create missing, drop closed.
        let open = self.layout.views();
        let closed: Vec<ViewRef> = self.views.keys().filter(|r| !open.contains(*r)).cloned().collect();
        // Views already gone need no watching: the list only holds possible leaks, so
        // it stays as small as the leaks themselves.
        self.released.retain(|(weak, _)| weak.upgrade().is_some());
        for r in closed {
            self.focus.remove(&r);
            if let Some(v) = self.views.remove(&r) {
                self.released.push((v.downgrade(), self.generation));
            }
        }
        for r in open {
            if !self.views.contains_key(&r) {
                let deck = cx.weak_entity();
                let ctx = self.services.clone().map(|services| ViewCx { services, deck, view: r.clone() });
                let built = views::build(&r, ctx.as_ref(), cx);
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
        self.refresh_visibility(cx);
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
            d.refresh_visibility(cx);
        }
        Err(e) => {
            d.os_windows.remove(&id);
            d.report(format!("could not open a window: {e}"), cx);
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
