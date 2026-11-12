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

use crate::places::{self, Place};
use crate::views::{self, ViewCx};
use devdeck_core::view_addr::ViewAddr;
use crate::window_view::{WindowView, window_options};
use devdeck_core::layout::{Layout, LayoutError, MAIN_WINDOW, PaneId, ViewRef, WindowId};
use devdeck_core::layout_book::{BookError, LayoutBook, LayoutId};
use devdeck_core::state::StateStore;
use devdeck_ui::{ToastKind, toasts};
use gpui::{AnyView, AnyWeakView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Task, WindowHandle};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

/// How long after a change the layout is written.
pub const SAVE_DELAY: Duration = Duration::from_millis(1000);

/// How long a revealed tab and its pane flash in the accent ([`Deck::reveal`]).
pub const FLASH: Duration = Duration::from_millis(1000);

/// A tab flashing after a reveal: drawn fading while it's set, cleared by a one-shot
/// timer (nothing runs once it's over, R1.3). `n` is fresh per flash: the drawing's
/// element id, so a second flash starts over.
#[derive(Clone, Debug, PartialEq)]
pub struct Flash {
    pub view: ViewRef,
    pub n: u64,
}

/// What [`Deck::reveal`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Revealed {
    /// It was open (in this layout or another, now shown): brought on screen.
    Shown,
    /// It was open nowhere: opened in the main window's focused pane ("Open here").
    OpenedHere,
}

/// Why a reveal did nothing ([`Deck::reveal`], [`crate::shell::Shell::reveal`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotRevealed {
    /// No workspace with that id any more (the caller says it).
    WorkspaceGone,
    /// Open nowhere, and its target is gone from the workspace (its app, command, env
    /// or note): nothing opened, logged.
    ViewGone(String),
    /// A dialog is open somewhere: it keeps the keys (the caller says to close it).
    Modal,
    /// The layout refused the change (the deck already said why).
    Refused,
}

/// Where a terminal shows ([`Deck::terminals`]).
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalTabs {
    /// The tab to show it by.
    pub view: ViewRef,
    /// The live layout shows it.
    pub here: bool,
    /// The saved layouts showing it: (id, name).
    pub layouts: Vec<(LayoutId, String)>,
}

/// What a live view said last: its title and its actions.
type Said = (Option<String>, Vec<views::ViewAction>, Option<views::ViewStatus>);

pub struct Deck {
    /// Read through [`Deck::layout`]; every change goes through [`Deck::apply`].
    layout: Layout,
    pub workspace: String,
    active: bool,
    views: HashMap<ViewRef, AnyView>,
    /// Focus handles of views that take keys.
    focus: HashMap<ViewRef, FocusHandle>,
    /// How views that keep state are read for saving.
    savers: HashMap<ViewRef, views::Saver>,
    /// Views that name themselves or offer actions ([`views::Live`]), with what they said
    /// last (a change redraws; typing in a note doesn't).
    lives: HashMap<ViewRef, (views::Lively, Said)>,
    /// States saved last time, handed to each view when it's built (then dropped).
    restored: devdeck_core::state::ViewStates,
    /// The views' states as last written (or about to be).
    written: devdeck_core::state::ViewStates,
    /// Views built as a message because their target was missing, with the state saved
    /// for them (kept, and handed over when they're built again).
    missing: HashMap<ViewRef, Option<serde_json::Value>>,
    /// The catalog the views were last built against.
    catalog: Option<std::rc::Rc<devdeck_core::catalog::Catalog>>,
    _services: Option<gpui::Subscription>,
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
    /// A "Send to" pick in progress (R3.11).
    pub send: Option<SendSource>,
    /// Named layouts of the workspace (R3.10).
    pub book: LayoutBook,
    /// Views on screen now; changes are announced with [`DeckEvent::Visibility`].
    visible: HashSet<ViewRef>,
    /// A full view's details (Apps): what the main window shows instead of the pulse's
    /// panes, the selection then what it was drilled into, the last on screen. `None`:
    /// the panes show.
    outside: Option<Vec<ViewRef>>,
    /// While a quiet change syncs: the OS windows it opens don't take the focus.
    opening_quietly: bool,
    /// Detached windows to open without taking the focus.
    open_unfocused: HashSet<WindowId>,
    /// The tab flashing after a reveal, if one is.
    pub flash: Option<Flash>,
    flashes: u64,
    flash_task: Option<Task<()>>,
}

/// Where the user's keys are in one window, or will be again when it shows the deck
/// (Settings closed, the workspace shown): a pane, and the tab it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub window: WindowId,
    pub pane: PaneId,
    pub view: Option<ViewRef>,
}

/// Every place the user's keys are in a deck ([`crate::window_view::keys`]).
pub type Keys = Vec<Held>;

/// Whether `h` is still where the keys were: its window's focused pane, showing the same
/// tab, and not hidden unless it already was.
fn keeps(before: &Layout, next: &Layout, h: &Held) -> bool {
    let shown = |l: &Layout| {
        l.window(h.window).is_ok_and(|w| w.maximized.is_none_or(|m| m == h.pane)) && l.pane(h.pane).is_ok_and(|p| !p.collapsed)
    };
    next.window(h.window).is_ok_and(|w| w.focused == h.pane)
        && next.pane(h.pane).is_ok_and(|p| p.active_view() == h.view.as_ref())
        && (shown(next) || !shown(before))
}

/// Why a quiet change was refused ([`Deck::apply_quietly`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Quiet {
    /// The layout refused it, for this reason.
    Refused(String),
    /// It would hide or move the view the user is typing in.
    Keys,
}

pub enum DeckEvent {
    /// Views that came on screen and views that left it, since the last change.
    Visibility { shown: Vec<ViewRef>, hidden: Vec<ViewRef> },
}

impl EventEmitter<DeckEvent> for Deck {}

/// What a "Send to" pick moves.
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
            savers: HashMap::new(),
            lives: HashMap::new(),
            restored: Default::default(),
            written: Default::default(),
            missing: HashMap::new(),
            catalog: None,
            _services: None,
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
            outside: None,
            opening_quietly: false,
            open_unfocused: HashSet::new(),
            flash: None,
            flashes: 0,
            flash_task: None,
        }
    }

    /// Ties the deck to a workspace: where it saves, and whether it starts active.
    pub fn in_workspace(mut self, id: impl Into<String>, store: Option<StateStore>, active: bool) -> Self {
        self.workspace = id.into();
        self.store = store;
        self.active = active;
        self
    }

    /// A number for a new terminal: above every terminal of the layouts (the book's
    /// included) and of the runner.
    pub fn next_terminal(&self, cx: &App) -> u32 {
        let number = |r: &ViewRef| match ViewAddr::parse(r) {
            ViewAddr::Terminal { n, .. } => Some(n),
            _ => None,
        };
        let in_layouts = std::iter::once(&self.layout).chain(self.book.layouts.values().filter_map(|n| n.layout.as_ref())).flat_map(|l| l.views()).filter_map(|r| number(&r));
        let in_runner = self.services.iter().flat_map(|s| s.read(cx).runner.read(cx).terminal_numbers());
        // An agent session keeps its number while its process comes and goes.
        let in_agents = self.services.iter().filter_map(|s| s.read(cx).agents.read(cx).max_terminal());
        in_layouts.chain(in_runner).chain(in_agents).max().map_or(1, |n: u32| n + 1)
    }

    /// Every terminal the layouts or the runner know, by number (R6.12): the tab to show
    /// it by (a layout's own ref, the live layout's first; else one from the runner's
    /// folder), whether the live layout shows it, and the saved layouts that do. The
    /// book's copy of the current layout is stale (the live one stands for it): skipped.
    pub fn terminals(&self, cx: &App) -> BTreeMap<u32, TerminalTabs> {
        let mut out: BTreeMap<u32, TerminalTabs> = BTreeMap::new();
        let number = |r: &ViewRef| match ViewAddr::parse(r) {
            ViewAddr::Terminal { n, .. } => Some(n),
            _ => None,
        };
        for r in self.layout.views() {
            if let Some(n) = number(&r) {
                out.entry(n).or_insert(TerminalTabs { view: r, here: true, layouts: Vec::new() });
            }
        }
        for (id, named) in self.book.layouts.iter().filter(|(id, _)| **id != self.book.current) {
            let Some(layout) = &named.layout else { continue };
            for r in layout.views() {
                let Some(n) = number(&r) else { continue };
                let t = out.entry(n).or_insert_with(|| TerminalTabs { view: r.clone(), here: false, layouts: Vec::new() });
                if !t.layouts.iter().any(|(i, _)| i == id) {
                    t.layouts.push((id.clone(), named.name.clone()));
                }
            }
        }
        if let Some(services) = &self.services {
            for (key, dir) in services.read(cx).runner.read(cx).terminals() {
                if let Some(n) = key.terminal_number() {
                    out.entry(n).or_insert_with(|| TerminalTabs { view: ViewAddr::Terminal { n, dir: dir.display().to_string() }.to_ref(), here: false, layouts: Vec::new() });
                }
            }
        }
        out
    }

    /// Tells the runner which terminals the layouts show: one whose shell isn't running
    /// and that no tab shows any more is forgotten (R6.12).
    fn report_terminals(&self, cx: &mut Context<Self>) {
        let Some(services) = &self.services else { return };
        let mut shown: std::collections::BTreeSet<u32> = self.terminals(cx).into_iter().filter(|(_, t)| t.here || !t.layouts.is_empty()).map(|(n, _)| n).collect();
        // A full view's details show terminals too.
        shown.extend(self.outside.iter().flatten().filter_map(|r| match ViewAddr::parse(r) {
            ViewAddr::Terminal { n, .. } => Some(n),
            _ => None,
        }));
        // An agent session's terminal, while a tab or the Agents view shows it: ended, it
        // stays with its tabs (⏎ resumes it).
        let agents = services.read(cx).agents.read(cx);
        let layouts = std::iter::once(&self.layout).chain(self.book.layouts.values().filter_map(|n| n.layout.as_ref()));
        let refs = layouts.flat_map(|l| l.views()).chain(self.outside.iter().flatten().cloned());
        shown.extend(refs.filter_map(|r| match ViewAddr::parse(&r) {
            ViewAddr::Agent { id } => agents.session(&id).map(|s| s.n),
            _ => None,
        }));
        let runner = services.read(cx).runner.clone();
        runner.update(cx, |r, cx| r.set_shown_terminals(shown, cx));
    }

    /// Opens a new terminal in `dir`, at `place`. Returns the pane showing it.
    pub fn open_terminal(&mut self, dir: &std::path::Path, place: Place, cx: &mut Context<Self>) -> Option<PaneId> {
        let addr = ViewAddr::Terminal { n: self.next_terminal(cx), dir: dir.display().to_string() };
        self.apply(cx, |l| places::open(l, addr.to_ref(), place))
    }

    /// A new session of agent `agent` in `dir` (R6.14), shown at `place`. Returns the
    /// pane showing it.
    pub fn open_agent(&mut self, agent: &str, dir: &std::path::Path, place: Place, cx: &mut Context<Self>) -> Option<PaneId> {
        let services = self.services.clone()?;
        let n = self.next_terminal(cx);
        let agents = services.read(cx).agents.clone();
        let id = agents.update(cx, |a, cx| a.create(agent, dir, n, cx));
        self.apply(cx, |l| places::open(l, ViewAddr::Agent { id }.to_ref(), place))
    }

    /// Where a new terminal starts for `pane`: the folder of its active tab (see
    /// [`Deck::folder_of`]); the main window's focused pane when `None`.
    pub fn terminal_folder(&self, pane: Option<PaneId>, cx: &App) -> std::path::PathBuf {
        let active = pane.or(Some(self.layout.main().focused)).and_then(|p| self.layout.pane(p).ok()?.active_view().cloned());
        self.folder_of(active.as_ref(), cx)
    }

    /// The folder of a view: its app's (an app or a run tab) or a terminal's own, else
    /// the workspace's first folder, else the home folder.
    pub fn folder_of(&self, view: Option<&ViewRef>, cx: &App) -> std::path::PathBuf {
        let home = || std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_else(|| "/".into());
        let Some(services) = &self.services else { return home() };
        let s = services.read(cx);
        let app = view.and_then(|r| match ViewAddr::parse(r) {
            ViewAddr::App { app } | ViewAddr::Run { app, .. } => s.catalog.app(&app).map(|a| a.dir.clone()),
            ViewAddr::Terminal { dir, .. } => Some(dir.into()),
            _ => None,
        });
        app.or_else(|| s.config.as_ref()?.folders.first().cloned()).unwrap_or_else(home)
    }

    /// The folder that is `view`'s own, where a shell or a session of it starts: an app's,
    /// a terminal's, an agent session's; `None` for the others (a note, a service, a port, a
    /// folder of apps).
    pub fn own_folder(&self, view: &ViewRef, cx: &App) -> Option<std::path::PathBuf> {
        let s = self.services.as_ref()?.read(cx);
        match ViewAddr::parse(view) {
            ViewAddr::App { app } => s.catalog.app(&app).map(|a| a.dir.clone()),
            ViewAddr::Terminal { dir, .. } => Some(dir.into()),
            ViewAddr::Agent { id } => s.agents.read(cx).session(&id).map(|a| a.dir.clone()),
            _ => None,
        }
    }

    /// Whether it belongs to a workspace (views other than the demo's need one).
    pub fn has_services(&self) -> bool {
        self.services.is_some()
    }

    /// Its workspace's services.
    pub fn services(&self) -> Option<Entity<crate::services::Services>> {
        self.services.clone()
    }

    pub fn with_services(mut self, services: Entity<crate::services::Services>) -> Self {
        self.services = Some(services);
        self
    }

    /// Builds the views whose target was missing again whenever the workspace's apps
    /// change (a project came back, a command was added).
    pub fn follow_services(&mut self, cx: &mut Context<Self>) {
        let Some(services) = self.services.clone() else { return };
        self.catalog = Some(services.read(cx).catalog.clone());
        self._services = Some(cx.observe(&services, |d, s, cx| {
            let catalog = s.read(cx).catalog.clone();
            if d.catalog.as_ref().is_some_and(|c| std::rc::Rc::ptr_eq(c, &catalog)) {
                return;
            }
            d.catalog = Some(catalog);
            d.rebuild_missing(cx);
        }));
    }

    fn rebuild_missing(&mut self, cx: &mut Context<Self>) {
        if self.missing.is_empty() {
            return;
        }
        for (r, saved) in std::mem::take(&mut self.missing) {
            self.focus.remove(&r);
            self.views.remove(&r);
            if let Some(state) = saved {
                self.restored.insert(r.as_str().to_string(), state);
            }
        }
        self.sync(cx);
        cx.notify();
    }

    /// The OS window showing detached window `w`, once open.
    pub fn os_window(&self, w: WindowId) -> Option<WindowHandle<WindowView>> {
        self.os_windows.get(&w).copied().flatten()
    }

    /// Where keys go when the active view of `pane` should have them.
    pub fn view_focus(&self, pane: PaneId) -> Option<FocusHandle> {
        let r = self.layout.pane(pane).ok()?.active_view()?;
        self.focus.get(r).cloned()
    }

    /// Whether a view is on screen: the shown workspace, its window drawn, the active tab of an expanded pane, not hidden by a maximized one.
    pub fn is_visible(&self, r: &ViewRef) -> bool {
        self.visible.contains(r)
    }

    fn compute_visible(&self) -> HashSet<ViewRef> {
        if !self.active {
            return HashSet::new();
        }
        let l = &self.layout;
        let panes_drawn = |w: WindowId| if w == MAIN_WINDOW { self.outside.is_none() } else { matches!(self.os_windows.get(&w), Some(Some(_))) };
        let mut shown: HashSet<ViewRef> = l.windows.iter().filter(|w| panes_drawn(w.id)).flat_map(|w| l.shown_in(w.id)).collect();
        shown.extend(self.outside.as_ref().and_then(|s| s.last()).cloned());
        shown
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
        self.report_agents(cx);
        cx.emit(DeckEvent::Visibility { shown, hidden });
    }

    /// Tells the agent sessions which of them are on screen (R6.14): a session runs
    /// while one is.
    fn report_agents(&self, cx: &mut Context<Self>) {
        let Some(services) = &self.services else { return };
        let ids: std::collections::BTreeSet<String> = self
            .visible
            .iter()
            .filter_map(|r| match ViewAddr::parse(r) {
                ViewAddr::Agent { id } => Some(id),
                _ => None,
            })
            .collect();
        let agents = services.read(cx).agents.clone();
        cx.defer(move |cx| agents.update(cx, |a, cx| a.set_shown(ids, cx)));
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// A full view's details, from the selection to what's on screen (`None`: the
    /// pulse's panes show).
    pub fn outside(&self) -> Option<&[ViewRef]> {
        self.outside.as_deref()
    }

    /// Shows a full view's details instead of the pulse's panes: `base` (the selection;
    /// none, an empty stack) and, when it's already the base, what it was drilled into.
    pub fn show_outside(&mut self, base: Option<ViewRef>, cx: &mut Context<Self>) {
        let stack: Vec<ViewRef> = base.into_iter().collect();
        match &self.outside {
            Some(now) if now.first() == stack.first() => return,
            _ => self.outside = Some(stack),
        }
        self.sync(cx);
        cx.notify();
    }

    /// Where keys go in the details: the view on top, when it takes keys.
    pub fn outside_focus(&self) -> Option<FocusHandle> {
        self.outside.as_ref()?.last().and_then(|r| self.focus.get(r).cloned())
    }

    /// Shows the pulse's panes again.
    pub fn hide_outside(&mut self, cx: &mut Context<Self>) {
        if self.outside.take().is_some() {
            self.sync(cx);
            cx.notify();
        }
    }

    /// Opens `to` over `from` when `from` is in the details: what was over `from` goes,
    /// `to` shows (already there, back to it). False when `from` isn't in the details.
    pub fn drill(&mut self, from: &ViewRef, to: ViewRef, cx: &mut Context<Self>) -> bool {
        let Some(stack) = self.outside.as_mut() else { return false };
        let Some(at) = stack.iter().position(|r| r == from) else { return false };
        match stack.iter().position(|r| *r == to) {
            Some(i) => stack.truncate(i + 1),
            None => {
                stack.truncate(at + 1);
                stack.push(to);
            }
        }
        self.sync(cx);
        cx.notify();
        true
    }

    /// Back to the details' `n`th view (0: the selection).
    pub fn back_to(&mut self, n: usize, cx: &mut Context<Self>) {
        if let Some(stack) = self.outside.as_mut()
            && n + 1 < stack.len()
        {
            stack.truncate(n + 1);
            self.sync(cx);
            cx.notify();
        }
    }

    /// A tab's title: what its view says, else what its address says.
    pub fn title(&self, r: &ViewRef, cx: &App) -> String {
        if let Some(t) = self.lives.get(r).and_then(|(l, _)| l.title(cx)) {
            return t;
        }
        if let (ViewAddr::Agent { id }, Some(s)) = (ViewAddr::parse(r), &self.services) {
            return s.read(cx).agents.read(cx).label(&id, cx);
        }
        views::title(r)
    }

    /// What a view says of its state now, for its one header (R9.11).
    pub fn view_status(&self, r: &ViewRef, cx: &App) -> Option<views::ViewStatus> {
        self.lives.get(r).and_then(|(l, _)| l.status(cx))
    }

    /// What a view offers now (R12.4), for its header and its tab's menu.
    pub fn view_actions(&self, r: &ViewRef, cx: &App) -> Vec<views::ViewAction> {
        self.lives.get(r).map(|(l, _)| l.actions(cx)).unwrap_or_default()
    }

    /// How to run a view's actions on it (R12.4).
    pub fn lively(&self, r: &ViewRef) -> Option<views::Lively> {
        self.lives.get(r).map(|(l, _)| l.clone())
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
                self.report(describe(&e), cx);
                None
            }
        };
        self.settle(cx);
        out
    }

    /// A layout change the user didn't make (the control API, R13.1): no window's
    /// focused pane changes, no pane shows another tab but `target`, the panes in `keys`
    /// keep their tab and stay shown, and windows it opens don't take the focus (except on
    /// Linux, see [`open_os_window`]). A refusal is the caller's to hear, not shown to the
    /// user.
    pub fn apply_quietly<T>(
        &mut self,
        keys: &Keys,
        target: Option<PaneId>,
        cx: &mut Context<Self>,
        op: impl FnOnce(&mut Layout) -> Result<T, LayoutError>,
    ) -> Result<T, Quiet> {
        let mut next = self.layout.clone();
        let out = next.in_background(target, op).map_err(|e| Quiet::Refused(describe(&e)))?;
        if !keys.iter().all(|h| keeps(&self.layout, &next, h)) {
            return Err(Quiet::Keys);
        }
        self.layout = next;
        self.changed(cx);
        self.opening_quietly = true;
        self.settle(cx);
        self.opening_quietly = false;
        Ok(out)
    }

    /// After a change: views and windows brought in line, observers told.
    fn settle(&mut self, cx: &mut Context<Self>) {
        debug_assert!(self.layout.validate().is_ok(), "{:?}", self.layout.validate());
        self.sync(cx);
        cx.notify();
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

    pub fn save_layout_as(&mut self, name: &str, cx: &mut Context<Self>) -> Result<LayoutId, BookError> {
        let live = self.layout.clone();
        let id = self.book.save_as(name, &live)?;
        self.save_book(cx);
        cx.notify();
        Ok(id)
    }

    /// Shows another named layout: tabs that aren't in it close (their runs go on),
    /// detached windows follow the layout.
    pub fn switch_layout(&mut self, id: &str, cx: &mut Context<Self>) -> Result<(), BookError> {
        if self.book.current == id {
            return Ok(());
        }
        let live = self.layout.clone();
        let target = self.book.switch(id, &live)?;
        self.show_layout(target, cx);
        Ok(())
    }

    /// Adds an empty layout under `name` and shows it; the one left keeps its tabs.
    pub fn new_layout(&mut self, name: &str, cx: &mut Context<Self>) -> Result<LayoutId, BookError> {
        let live = self.layout.clone();
        let (id, target) = self.book.new_layout(name, &live)?;
        self.show_layout(target, cx);
        Ok(id)
    }

    /// Puts `target` on screen in place of the live layout and writes it with the book.
    fn show_layout(&mut self, target: Layout, cx: &mut Context<Self>) {
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
            if let Some((store, ws, layout, views)) = self.take_save(cx) {
                let result = store.save_layout(&ws, &layout).and(store.save_views(&ws, &views));
                self.saved(result, cx);
            }
            self.save_book(cx);
        }
    }

    /// Whether `view` is open in this workspace: in the live layout or a saved one.
    pub fn holds(&self, view: &ViewRef) -> bool {
        self.layout.find_view(view).is_some() || self.layout_holding(view).is_some()
    }

    /// Why `view` can't be revealed: open in no layout, and its target gone from the
    /// workspace ([`views::gone`]).
    pub fn gone(&self, view: &ViewRef, cx: &App) -> Option<String> {
        if self.holds(view) {
            return None;
        }
        self.services.as_ref().and_then(|s| views::gone(view, s.read(cx), cx))
    }

    /// The saved layout (not the current one) showing `view`, the first by name.
    fn layout_holding(&self, view: &ViewRef) -> Option<LayoutId> {
        let mut holding: Vec<(&LayoutId, &str)> = self
            .book
            .layouts
            .iter()
            .filter(|(id, n)| **id != self.book.current && n.layout.as_ref().is_some_and(|l| l.find_view(view).is_some()))
            .map(|(id, n)| (id, n.name.as_str()))
            .collect();
        holding.sort_by_key(|(_, name)| name.to_lowercase());
        holding.first().map(|(id, _)| (*id).clone())
    }

    /// Brings `view` on screen (an "Open" from a notification, R2.2, R3.8): where it is in
    /// the live layout, else in the saved layout holding it (switched to: the layout left
    /// keeps its tabs and windows), its tab active, its pane expanded and focused, a pane
    /// maximized over it restored; open nowhere, it opens in the main window's focused
    /// pane. Then its tab flashes. Returns what it did and the window showing it (the
    /// caller brings that window forward).
    pub fn reveal(&mut self, view: &ViewRef, cx: &mut Context<Self>) -> Result<(Revealed, WindowId), NotRevealed> {
        if self.layout.find_view(view).is_none() {
            match self.layout_holding(view) {
                Some(id) => {
                    if let Err(e) = self.switch_layout(&id, cx) {
                        log::warn!("reveal {}: {e}", view.0);
                    }
                }
                None => {
                    if let Some(why) = self.gone(view, cx) {
                        log::warn!("reveal {}: {why}", view.0);
                        return Err(NotRevealed::ViewGone(why));
                    }
                }
            }
        }
        let (pane, open) = self.apply(cx, |l| l.reveal(view.clone())).ok_or(NotRevealed::Refused)?;
        let window = self.layout.window_of(pane).map_err(|e| {
            log::warn!("reveal {}: {e}", view.0);
            NotRevealed::Refused
        })?;
        self.flash(view.clone(), cx);
        Ok((if open { Revealed::Shown } else { Revealed::OpenedHere }, window))
    }

    /// Flashes `view`'s tab and pane for [`FLASH`]; a flash already running gives way.
    pub fn flash(&mut self, view: ViewRef, cx: &mut Context<Self>) {
        self.flashes += 1;
        let n = self.flashes;
        self.flash = Some(Flash { view, n });
        self.flash_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLASH).await;
            let _ = this.update(cx, |d, cx| {
                if d.flash.as_ref().is_some_and(|f| f.n == n) {
                    d.flash = None;
                    d.flash_task = None;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    /// Opens `view` in pulse `id`'s first pane (its main window's first, in tree order):
    /// the live pulse as any change, a saved one in its book. Returns the pulse's name.
    pub fn open_in_pulse(&mut self, id: &str, view: ViewRef, cx: &mut Context<Self>) -> Result<String, String> {
        let name = self.book.name(id).ok_or_else(|| format!("no pulse {id}"))?.to_string();
        let first = |l: &Layout| l.panes_in(MAIN_WINDOW).ok().and_then(|p| p.first().copied()).unwrap_or(l.main().focused);
        if id == self.book.current {
            self.apply(cx, |l| {
                let p = first(l);
                l.open(view, p).map(|_| ())
            })
            .ok_or_else(|| self.last_error.clone().unwrap_or_else(|| "refused".into()))?;
        } else {
            let named = self.book.layouts.get_mut(id).ok_or_else(|| format!("no pulse {id}"))?;
            let mut l = named.layout.clone().unwrap_or_default();
            let p = first(&l);
            l.open(view, p).map_err(|e| e.to_string())?;
            named.layout = Some(l);
            self.save_book(cx);
            cx.notify();
        }
        Ok(name)
    }

    pub fn rename_layout(&mut self, id: &str, to: &str, cx: &mut Context<Self>) -> Result<(), BookError> {
        self.book.rename(id, to)?;
        self.save_book(cx);
        cx.notify();
        Ok(())
    }

    pub fn delete_layout(&mut self, id: &str, cx: &mut Context<Self>) -> Result<(), BookError> {
        self.book.delete(id)?;
        self.save_book(cx);
        cx.notify();
        Ok(())
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
        // A deleted layout may have been the last to show a stopped terminal.
        self.report_terminals(cx);
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
        // Views: create missing, drop closed (the details are kept like tabs).
        let mut open = self.layout.views();
        open.extend(self.outside.iter().flatten().cloned());
        let closed: Vec<ViewRef> = self.views.keys().filter(|r| !open.contains(*r)).cloned().collect();
        // Views already gone need no watching: the list only holds possible leaks, so
        // it stays as small as the leaks themselves.
        self.released.retain(|(weak, _)| weak.upgrade().is_some());
        for r in closed {
            // A terminal's shell outlives its tabs (R6.12): its view only goes.
            self.focus.remove(&r);
            self.savers.remove(&r);
            self.lives.remove(&r);
            self.missing.remove(&r);
            if let Some(v) = self.views.remove(&r) {
                self.released.push((v.downgrade(), self.generation));
            }
        }
        for r in open {
            if !self.views.contains_key(&r) {
                let deck = cx.weak_entity();
                let ctx = self.services.clone().map(|services| ViewCx { services, deck, view: r.clone() });
                let saved = self.restored.remove(r.as_str());
                let built = views::build(&r, ctx.as_ref(), saved.as_ref(), cx);
                if let Some(f) = built.focus {
                    self.focus.insert(r.clone(), f);
                }
                if let Some(s) = built.save {
                    self.savers.insert(r.clone(), s);
                }
                if let Some(l) = built.live {
                    let said = (l.title(cx), l.actions(cx), l.status(cx));
                    self.lives.insert(r.clone(), (l, said));
                }
                if built.missing {
                    self.missing.insert(r.clone(), saved);
                }
                self.views.insert(r, built.view);
            }
        }
        self.report_terminals(cx);
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
                if self.opening_quietly {
                    self.open_unfocused.insert(id);
                }
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

    /// A view that keeps state changed: saved soon if its state did.
    pub fn view_changed(&mut self, r: &ViewRef, cx: &mut Context<Self>) {
        // Its title or header may have changed.
        if let Some((l, said)) = self.lives.get_mut(r) {
            let now = (l.title(cx), l.actions(cx), l.status(cx));
            if *said != now {
                *said = now;
                cx.notify();
            }
        }
        let Some(save) = self.savers.get(r) else { return };
        if save(cx).as_ref() != self.written.get(r.as_str()) && self.store.is_some() && self.save_task.is_none() {
            self.schedule_save(cx);
        }
    }

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
            let Ok(Some((store, ws, layout, views))) = this.update(cx, |d, cx| d.take_save(cx)) else { return };
            let result = cx.background_executor().spawn(async move { store.save_layout(&ws, &layout).and(store.save_views(&ws, &views)) }).await;
            let _ = this.update(cx, |d, cx| {
                d.save_task = None;
                d.saved(result, cx);
                if d.dirty {
                    d.schedule_save(cx);
                }
            });
        }));
    }

    fn take_save(&mut self, cx: &App) -> Option<(StateStore, String, Layout, devdeck_core::state::ViewStates)> {
        let store = self.store.clone()?;
        self.dirty = false;
        self.saves += 1;
        self.written = self.view_states(cx);
        Some((store, self.workspace.clone(), self.layout.clone(), self.written.clone()))
    }

    /// What the open views keep of themselves, by view ref.
    pub fn view_states(&self, cx: &App) -> devdeck_core::state::ViewStates {
        let missing = self.missing.iter().filter_map(|(r, saved)| Some((r.as_str().to_string(), saved.clone()?)));
        self.savers.iter().filter_map(|(r, save)| Some((r.as_str().to_string(), save(cx)?))).chain(missing).collect()
    }

    /// The views' states saved last time (applied as each view is built).
    pub fn with_view_states(mut self, states: devdeck_core::state::ViewStates) -> Self {
        self.written = states.clone();
        self.restored = states;
        self
    }

    /// A failing save is reported once, until a save works again.
    fn saved(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        match result {
            Ok(()) => self.save_failed = false,
            Err(e) if !self.save_failed => {
                self.save_failed = true;
                let text = format!("Could not save the {} pulse: {e}", self.workspace);
                toasts(cx).update(cx, |t, cx| t.push(ToastKind::Err, text, vec![], cx));
            }
            Err(_) => {}
        }
    }

    /// Writes everything now (quit): the layout when it changed, the views' states always
    /// (they change without the layout: scrolling, a run starting).
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        self.save_task = None;
        let dirty = self.dirty;
        if let Some((store, ws, layout, views)) = self.take_save(cx) {
            let result = if dirty { store.save_layout(&ws, &layout) } else { Ok(()) };
            self.saved(result.and(store.save_views(&ws, &views)), cx);
        }
    }
}

fn open_os_window(deck: Entity<Deck>, id: WindowId, cx: &mut App) {
    // GPUI's Linux backends don't honour `focus` (0.3.11): there the compositor decides.
    let quiet = deck.update(cx, |d, _| d.open_unfocused.remove(&id));
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
    let options = gpui::WindowOptions { focus: !quiet, ..window_options(size, false, cx) };
    let result = cx.open_window(options, |window, cx| cx.new(|cx| WindowView::detached(id, view_deck, window, cx)));
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
