//! The sidebar of one workspace (R4): the activity bar's choice (Apps, Cluster, Ports)
//! and its list, Pinned then All, alphabetical, with a filter. Every workspace owns its
//! own `Sidebar`; the main window shows the active one.
//!
//! Selection is keyed by item, so it survives filtering and reordering; an item that
//! gets hidden drops the selection. Rows open their item as a tab in the main window's
//! focused pane (or wherever they are dropped), and pin or unpin with `p`, the menu, or a
//! drop on the other section.
//!
//! Apps are a tree of their folders (`list::rows`): → opens a row (or goes to its first
//! child), ← closes it (or goes to its parent), Enter or a double click toggles a folder,
//! the chevron toggles any row. What's open is saved with the sidebar; while filtering,
//! the matches show with their folders open and the saved state is left as it was.

use crate::deck::Deck;
use crate::places::{self, Place};
use crate::runner::RunKey;
use crate::services::Services;
use devdeck_core::cluster::{Env, Remote};
use devdeck_core::view_addr::ViewAddr;
use devdeck_core::layout::{Drop, Layout, MAIN_WINDOW, Result as LayoutResult, ViewRef};
use devdeck_core::list::{self, Kids, ListItem, Row, Section};
use devdeck_core::state::{SidebarState, StateStore};
use devdeck_ui::{DragGhost, Icon, InputEvent, MenuItem, TextInput, Theme, ToastKind, dot, icon, label, open_menu, theme, tip, toasts, toggle_button, tree_row};
use gpui::{
    App, AppContext, Context, Entity, EntityId, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription,
    UniformListScrollHandle, Window, actions, div, prelude::*, uniform_list,
};

actions!(
    sidebar,
    [
        SelectNext,
        SelectPrev,
        SelectFirst,
        SelectLast,
        OpenSelected,
        ExpandSelected,
        CollapseSelected,
        TogglePin,
        FocusFilter,
        ToggleRunningOnly,
        SidebarEscape
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Apps,
    Cluster,
    Ports,
    Layouts,
    /// Every shell of the workspace, shown or not.
    Terminals,
}

impl Activity {
    pub const ALL: [Activity; 5] = [Activity::Apps, Activity::Cluster, Activity::Ports, Activity::Layouts, Activity::Terminals];

    pub fn id(self) -> &'static str {
        match self {
            Activity::Apps => "apps",
            Activity::Cluster => "cluster",
            Activity::Ports => "ports",
            Activity::Layouts => "layouts",
            Activity::Terminals => "terminals",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    /// The pin set it uses. Deployment pins are shared by every env.
    fn list_id(self) -> &'static str {
        match self {
            Activity::Apps => "apps",
            Activity::Cluster => "deployments",
            Activity::Ports => "ports",
            Activity::Layouts => "layouts",
            Activity::Terminals => "terminals",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Activity::Apps => "Apps",
            Activity::Cluster => "Cluster",
            Activity::Ports => "Ports",
            Activity::Layouts => "Layouts",
            Activity::Terminals => "Terminals",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Activity::Apps => Icon::Apps,
            Activity::Cluster => Icon::Cluster,
            Activity::Ports => Icon::Ports,
            Activity::Layouts => Icon::Layouts,
            Activity::Terminals => Icon::Terminals,
        }
    }
}

/// A list row being dragged: onto the other section to pin/unpin, onto a pane to open.
#[derive(Clone)]
pub struct DraggedListItem {
    pub sidebar: EntityId,
    /// The deck the item opens in.
    pub deck: EntityId,
    pub key: String,
    pub label: SharedString,
    pub pinned: bool,
    pub view: ViewRef,
}

/// Opens `view` at `drop` (into a pane or at its edge); an open view moves there.
pub fn open_at(l: &mut Layout, view: ViewRef, drop: Drop) -> LayoutResult<()> {
    if let Some((p, i)) = l.find_view(&view) {
        return l.move_tab(p, i, drop);
    }
    let pane = l.open(view.clone(), drop.pane())?;
    if let Drop::Edge(..) = drop {
        let i = l.pane(pane)?.tabs.iter().position(|r| *r == view).expect("just opened");
        l.move_tab(pane, i, drop)?;
    }
    Ok(())
}

pub struct Sidebar {
    pub workspace: String,
    pub services: Entity<Services>,
    deck: Entity<Deck>,
    store: Option<StateStore>,
    pub state: SidebarState,
    pub running_only: bool,
    pub filter: Entity<TextInput>,
    pub list_focus: FocusHandle,
    pub selected: Option<String>,
    scroll: UniformListScrollHandle,
    save_failed: bool,
    /// While the Terminals list shows: redraws every [`TERMINALS_TICK`] (what a shell
    /// runs changes without anyone saying so).
    tick: Option<gpui::Task<()>>,
    _subs: Vec<Subscription>,
}

/// How often the Terminals list looks at what its shells run.
pub const TERMINALS_TICK: std::time::Duration = std::time::Duration::from_secs(1);

impl Sidebar {
    pub fn new(
        workspace: String,
        services: Entity<Services>,
        deck: Entity<Deck>,
        store: Option<StateStore>,
        mut state: SidebarState,
        cx: &mut Context<Self>,
    ) -> Self {
        // Repair what the workspace can't honour (no cluster, unknown activity); the
        // services already settled the env.
        let has_cluster = services.read(cx).cluster.is_some();
        let activity = Activity::from_id(&state.activity).unwrap_or(Activity::Apps);
        let activity = if activity == Activity::Cluster && !has_cluster { Activity::Apps } else { activity };
        state.activity = activity.id().into();
        state.env = services.read(cx).env.clone();
        let filter = cx.new(|cx| TextInput::new("filter", cx));
        let subs = vec![
            // The Layouts list reads the deck's book.
            cx.observe(&deck, |_, _, cx| cx.notify()),
            cx.observe(&services, |this, _, cx| {
                this.drop_hidden_selection(cx);
                cx.notify();
            }),
            cx.subscribe(&filter, |this, _, e: &InputEvent, cx| match e {
            InputEvent::Changed(_) => {
                this.drop_hidden_selection(cx);
                cx.notify();
            }
            InputEvent::Submit(_) => this.open_selected(cx),
            InputEvent::Cancel => {}
        }),
        ];
        let this = Self {
            workspace,
            services,
            deck,
            store,
            state,
            running_only: false,
            filter,
            list_focus: cx.focus_handle(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            save_failed: false,
            tick: None,
            _subs: subs,
        };
        this.load_list(cx);
        this
    }

    /// Starts loading what the shown list needs (the env's deployments).
    fn load_list(&self, cx: &mut App) {
        if self.activity() != Activity::Cluster {
            return;
        }
        let s = self.services.read(cx);
        if let (Some(cluster), Some(env)) = (s.cluster.clone(), s.env.clone()) {
            cluster.update(cx, |c, cx| {
                c.deployments(&env, cx);
            });
        }
    }

    pub fn activity(&self) -> Activity {
        Activity::from_id(&self.state.activity).unwrap_or(Activity::Apps)
    }

    /// Activities this workspace offers (Cluster only with a cluster, R2.3).
    pub fn activities(&self, cx: &App) -> Vec<Activity> {
        let has_cluster = self.services.read(cx).cluster.is_some();
        Activity::ALL.into_iter().filter(|a| *a != Activity::Cluster || has_cluster).collect()
    }

    pub fn envs(&self, cx: &App) -> Vec<Env> {
        self.services.read(cx).catalog.envs.clone()
    }

    pub fn env(&self, cx: &App) -> Option<Env> {
        let name = self.services.read(cx).env.clone()?;
        self.envs(cx).into_iter().find(|e| e.name == name)
    }

    /// Why the list is empty when it is for a reason (loading, unreachable env).
    pub fn status(&self, cx: &App) -> Option<String> {
        if self.activity() != Activity::Cluster {
            return None;
        }
        let s = self.services.read(cx);
        match (s.cluster.as_ref(), s.env.as_deref()) {
            (Some(c), Some(env)) => match c.read(cx).peek(env) {
                Remote::Loading => Some("loading…".into()),
                Remote::Error(e) => Some(e),
                Remote::Ready(_) => None,
            },
            _ => None,
        }
    }

    pub fn items(&self, cx: &App) -> Vec<ListItem> {
        let s = self.services.read(cx);
        match self.activity() {
            Activity::Apps => {
                let runner = s.runner.read(cx);
                s.catalog
                    .apps
                    .iter()
                    .map(|a| {
                        let warning = s.unusable.get(&a.id).cloned();
                        ListItem::new(a.id.clone(), a.name.clone())
                            .detail(a.kind.clone())
                            .running(runner.app_running(&a.id))
                            .warning(warning)
                            .path(a.id.clone())
                            .dim(a.library)
                    })
                    .collect()
            }
            Activity::Cluster => match (s.cluster.as_ref(), s.env.as_deref()) {
                (Some(c), Some(env)) => {
                    let c = c.read(cx);
                    c.peek(env)
                        .ready()
                        .map(|deps| {
                            deps.iter()
                                .map(|d| {
                                    let rolling = c.rollout(env, &d.name).is_some_and(|r| matches!(r, devdeck_core::cluster::Rollout::Progressing { .. }));
                                    ListItem::new(d.name.clone(), d.name.clone()).detail(d.tag.clone()).busy(rolling).running(d.ready == d.replicas)
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
                _ => Vec::new(),
            },
            Activity::Ports => s
                .ports(cx)
                .into_iter()
                .map(|p| ListItem::new(p.port.to_string(), format!(":{}", p.port)).detail(p.holder).running(true))
                .collect(),
            Activity::Layouts => {
                let book = &self.deck.read(cx).book;
                book.names()
                    .into_iter()
                    .map(|n| {
                        let current = n == book.current;
                        ListItem::new(n.clone(), n).detail(if current { "current" } else { "" }).running(current)
                    })
                    .collect()
            }
            Activity::Terminals => {
                // Each shell, what it does, and where it shows: here, another layout, or
                // nowhere (still running, listed here so it isn't lost).
                let runner = s.runner.read(cx);
                let deck = self.deck.read(cx);
                let shows_in = |n: u32| -> String {
                    let is_it = |r: &ViewRef| matches!(ViewAddr::parse(r), ViewAddr::Terminal { n: m, .. } if m == n);
                    if deck.layout().views().iter().any(is_it) {
                        return "here".into();
                    }
                    let other = deck.book.layouts.iter().find(|(name, l)| **name != deck.book.current && l.views().iter().any(is_it));
                    other.map_or_else(|| "not shown".into(), |(name, _)| format!("in {name}"))
                };
                runner
                    .terminals()
                    .into_iter()
                    .filter_map(|(key, _)| {
                        let n = key.terminal_number()?;
                        let state = runner.state(&key);
                        let doing = match (runner.terminal_program(&key), state) {
                            (Some(program), _) => program,
                            (None, Some(s)) if s.is_running() => "idle".into(),
                            (None, Some(_)) => "ended".into(),
                            (None, None) => "not started".into(),
                        };
                        let folder = runner.title(&key).trim_start_matches("terminal · ").to_string();
                        let item = ListItem::new(n.to_string(), folder).detail(format!("{doing} · {}", shows_in(n)));
                        Some(item.running(state.is_some_and(|s| s.is_running())))
                    })
                    .collect()
            }
        }
    }

    fn pins(&self) -> std::collections::BTreeSet<String> {
        self.state.pins.get(self.activity().list_id()).cloned().unwrap_or_default()
    }

    /// The open tree rows of the shown list.
    fn opened(&self) -> std::collections::BTreeSet<String> {
        self.state.open.get(self.activity().list_id()).cloned().unwrap_or_default()
    }

    fn running_only(&self) -> bool {
        self.running_only && self.activity() == Activity::Apps
    }

    /// Filtering or "running only": the tree shows the matches, all open.
    fn narrowing(&self, cx: &App) -> bool {
        !self.filter.read(cx).text().trim().is_empty() || self.running_only()
    }

    pub fn rows(&self, cx: &App) -> (Vec<ListItem>, Vec<Row>) {
        let items = self.items(cx);
        let rows = list::rows(&items, &self.pins(), self.filter.read(cx).text(), self.running_only(), &self.opened());
        (items, rows)
    }

    fn selected_row(&self, items: &[ListItem], rows: &[Row]) -> Option<usize> {
        let key = self.selected.as_deref()?;
        rows.iter().position(|r| row_key(items, r).as_deref() == Some(key))
    }

    fn drop_hidden_selection(&mut self, cx: &App) {
        let (items, rows) = self.rows(cx);
        if self.selected_row(&items, &rows).is_none() {
            self.selected = None;
        }
    }

    fn select_row(&mut self, items: &[ListItem], rows: &[Row], row: Option<usize>, cx: &mut Context<Self>) {
        if let Some(r) = row
            && let Some(key) = row_key(items, &rows[r])
        {
            self.selected = Some(key);
            self.scroll.scroll_to_item(r, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    /// Opens or closes a tree row. While filtering everything shown is open: nothing to
    /// toggle. A selection inside a row that closes moves to it.
    pub fn toggle_open(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.narrowing(cx) {
            return;
        }
        let open = self.state.open.entry(self.activity().list_id().to_string()).or_default();
        let closing = open.remove(path);
        if !closing {
            open.insert(path.to_string());
        }
        let (items, rows) = self.rows(cx);
        if closing && self.selected.is_some() && self.selected_row(&items, &rows).is_none() {
            let row = rows.iter().position(|r| r.open_path(&items) == Some(path));
            self.select_row(&items, &rows, row, cx);
        }
        self.save(cx);
        cx.notify();
    }

    /// →: opens the selected row, or goes to its first child when it's open.
    fn expand_selected(&mut self, cx: &mut Context<Self>) {
        let (items, rows) = self.rows(cx);
        let Some(r) = self.selected_row(&items, &rows) else { return };
        match (rows[r].kids(), rows[r].open_path(&items)) {
            (Some(k), Some(path)) if !k.open => self.toggle_open(path, cx),
            (Some(_), _) => self.move_selection(1, cx),
            _ => {}
        }
    }

    /// ←: closes the selected row, or goes to its parent.
    fn collapse_selected(&mut self, cx: &mut Context<Self>) {
        let (items, rows) = self.rows(cx);
        let Some(r) = self.selected_row(&items, &rows) else { return };
        if let (Some(k), Some(path)) = (rows[r].kids(), rows[r].open_path(&items))
            && k.open
            && !self.narrowing(cx)
        {
            return self.toggle_open(path, cx);
        }
        let depth = rows[r].depth();
        let parent = (0..r).rev().find(|&i| rows[i].is_selectable() && rows[i].depth() < depth);
        if depth > 0 && parent.is_some() {
            self.select_row(&items, &rows, parent, cx);
        }
    }

    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let (items, rows) = self.rows(cx);
        let from = self.selected_row(&items, &rows);
        let to = list::step(&rows, from, delta);
        self.select_row(&items, &rows, to, cx);
    }

    fn select_edge(&mut self, last: bool, cx: &mut Context<Self>) {
        let (items, rows) = self.rows(cx);
        let to = list::step(&rows, None, 0).and_then(|first| if last { list::step(&rows, Some(first), isize::MAX / 2) } else { Some(first) });
        self.select_row(&items, &rows, to, cx);
    }

    /// The view an item opens as (`None` for layouts: they switch instead).
    pub fn view_ref(&self, key: &str, cx: &App) -> Option<ViewRef> {
        Some(match self.activity() {
            Activity::Apps => ViewAddr::App { app: key.into() },
            Activity::Cluster => ViewAddr::Svc { env: self.services.read(cx).env.clone().unwrap_or_default(), name: key.into() },
            Activity::Ports => ViewAddr::Port { port: key.into() },
            Activity::Layouts => return None,
            Activity::Terminals => {
                let n: u32 = key.parse().ok()?;
                let dir = self.services.read(cx).runner.read(cx).terminals().into_iter().find(|(k, _)| *k == RunKey::terminal(n))?.1;
                ViewAddr::Terminal { n, dir: dir.display().to_string() }
            }
        }
        .to_ref())
    }

    /// Ends terminal `n` everywhere (its tab, every layout, its shell).
    pub fn end_terminal(&mut self, n: u32, cx: &mut Context<Self>) {
        self.deck.update(cx, |d, cx| d.end_terminal(n, cx));
    }

    /// Opens an item in the main window's focused pane; an open one is activated.
    pub fn open(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(view) = self.view_ref(key, cx) else {
            let key = key.to_string();
            self.deck.update(cx, |d, cx| {
                let _ = d.switch_layout(&key, cx);
            });
            return;
        };
        self.deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let p = l.main().focused;
                l.open(view, p).map(|_| ())
            });
        });
    }

    /// Enter: opens the selected item, opens or closes the selected folder.
    pub fn open_selected(&mut self, cx: &mut Context<Self>) {
        let (items, rows) = self.rows(cx);
        if self.selected_row(&items, &rows).is_none() {
            // Nothing selected (e.g. right after typing a filter): take the first match,
            // not a folder or an app shown only to hold it.
            let filter = self.filter.read(cx).text().to_string();
            let first = rows.iter().position(|r| matches!(r, Row::Item { index, .. } if list::keeps(&items[*index], &filter, self.running_only())));
            self.select_row(&items, &rows, first, cx);
        }
        let Some(r) = self.selected_row(&items, &rows) else { return };
        match &rows[r] {
            Row::Folder { path, .. } => self.toggle_open(&path.clone(), cx),
            Row::Item { index, .. } => self.open(&items[*index].key.clone(), cx),
            _ => {}
        }
    }

    pub fn toggle_pin(&mut self, key: &str, cx: &mut Context<Self>) {
        // Terminals come and go with their numbers: nothing to pin.
        if self.activity() == Activity::Terminals {
            return;
        }
        let pins = self.state.pins.entry(self.activity().list_id().to_string()).or_default();
        list::toggle_pin(pins, key);
        self.save(cx);
        cx.notify();
    }

    fn toggle_pin_selected(&mut self, cx: &mut Context<Self>) {
        // Folders aren't pinned (their key ends with `/`).
        if let Some(key) = self.selected.clone().filter(|k| !k.ends_with('/')) {
            self.toggle_pin(&key, cx);
            let (items, rows) = self.rows(cx);
            let row = self.selected_row(&items, &rows);
            self.select_row(&items, &rows, row, cx);
        }
    }

    /// Shows an activity (and the sidebar); unknown or unavailable ones are ignored.
    /// Returns whether it is now shown.
    pub fn show(&mut self, activity: Activity, cx: &mut Context<Self>) -> bool {
        if !self.activities(cx).contains(&activity) {
            return false;
        }
        if self.activity() != activity {
            self.state.activity = activity.id().into();
            self.selected = None;
            self.filter.update(cx, |f, cx| f.set_text("", cx));
        }
        self.state.visible = true;
        self.load_list(cx);
        self.save(cx);
        cx.notify();
        true
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.state.visible != visible {
            self.state.visible = visible;
            self.save(cx);
            cx.notify();
        }
    }

    pub fn set_env(&mut self, env: &str, cx: &mut Context<Self>) {
        if self.envs(cx).iter().any(|e| e.name == env) && self.state.env.as_deref() != Some(env) {
            self.state.env = Some(env.into());
            self.services.update(cx, |s, cx| s.set_env(env, cx));
            self.load_list(cx);
            self.drop_hidden_selection(cx);
            self.save(cx);
            cx.notify();
        }
    }

    pub fn toggle_running_only(&mut self, cx: &mut Context<Self>) {
        if self.activity() == Activity::Apps {
            self.running_only = !self.running_only;
            self.drop_hidden_selection(cx);
            cx.notify();
        }
    }

    /// Esc: clears the filter first; with an empty filter the action goes on to the
    /// window, which sends focus back to the panes.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter.read(cx).text().is_empty() {
            cx.propagate();
        } else {
            self.filter.update(cx, |f, cx| f.set_text("", cx));
            self.list_focus.focus(window, cx);
            cx.notify();
        }
    }

    /// Writes the sidebar state in the background; a failure is reported once.
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else { return };
        let (ws, state) = (self.workspace.clone(), self.state.clone());
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { store.save_sidebar(&ws, &state) }).await;
            let _ = this.update(cx, |s, cx| match result {
                Ok(()) => s.save_failed = false,
                Err(e) if !s.save_failed => {
                    s.save_failed = true;
                    toasts(cx).update(cx, |t, cx| t.push(ToastKind::Err, format!("Could not save the sidebar: {e}"), vec![], cx));
                }
                Err(_) => {}
            });
        })
        .detach();
    }

    pub fn item_menu(this: &Entity<Self>, key: &str, pinned: bool, cx: &App) -> Vec<MenuItem> {
        let deck = this.read(cx).deck.clone();
        if this.read(cx).activity() == Activity::Terminals {
            let end = this.clone();
            let n = key.parse::<u32>().unwrap_or_default();
            let Some(view) = this.read(cx).view_ref(key, cx) else { return Vec::new() };
            // Opened where it shows, else in the main window's focused pane.
            let l = deck.read(cx).layout();
            let here = l.find_view(&view).map_or(l.main().focused, |(p, _)| p);
            let v = view.clone();
            return vec![
                places::item("Open", Place::Pane(here), MAIN_WINDOW, &deck, Some(&view), cx, move |d, place, cx| {
                    d.apply(cx, |l| places::open(l, v.clone(), place))
                }),
                MenuItem::new("End", move |_, cx| end.update(cx, |s, cx| s.end_terminal(n, cx))),
            ];
        }
        if this.read(cx).activity() == Activity::Layouts {
            let deck = this.read(cx).deck.clone();
            let current = deck.read(cx).book.current == key;
            let (d1, d2, d3) = (deck.clone(), deck.clone(), deck);
            let (k1, k2, k3) = (key.to_string(), key.to_string(), key.to_string());
            let mut delete = MenuItem::new("Delete", move |_, cx| {
                d3.update(cx, |d, cx| {
                    let _ = d.delete_layout(&k3, cx);
                })
            });
            if current {
                delete = delete.disabled("switch to another layout first");
            }
            return vec![
                MenuItem::new("Switch to it", move |_, cx| {
                    d1.update(cx, |d, cx| {
                        let _ = d.switch_layout(&k1, cx);
                    })
                }),
                MenuItem::new("Rename…", move |window, cx| crate::layouts::open_rename(d2.clone(), k2.clone(), window, cx)),
                delete,
            ];
        }
        let (open, pin) = (this.clone(), this.clone());
        let (k1, k2) = (key.to_string(), key.to_string());
        let mut items = vec![
            MenuItem::new("Open", move |_, cx| open.update(cx, |s, cx| s.open(&k1, cx))),
            MenuItem::new(if pinned { "Unpin" } else { "Pin" }, move |_, cx| pin.update(cx, |s, cx| s.toggle_pin(&k2, cx))),
        ];
        // An app's shell, in its folder (not a folder row's).
        if this.read(cx).activity() == Activity::Apps
            && this.read(cx).services.read(cx).catalog.app(key).is_some()
            && let Some(view) = this.read(cx).view_ref(key, cx)
        {
            let d = deck.read(cx);
            let (folder, focused) = (d.folder_of(Some(&view), cx), d.layout().main().focused);
            items.push(MenuItem::separator());
            items.push(places::item("Open terminal here", Place::Pane(focused), MAIN_WINDOW, &deck, None, cx, move |d, place, cx| {
                d.open_terminal(&folder, place, cx)
            }));
        }
        items
    }

    pub fn env_menu(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let s = this.read(cx);
        let current = s.state.env.clone();
        let err = theme(cx).err;
        s.envs(cx)
            .iter()
            .map(|e| {
                let (sidebar, name) = (this.clone(), e.name.clone());
                let item = MenuItem::new(e.name.clone(), move |_, cx| sidebar.update(cx, |s, cx| s.set_env(&name, cx))).checked(current.as_deref() == Some(&e.name));
                if e.protected { item.icon(Icon::Protected, err) } else { item }
            })
            .collect()
    }
}

impl Focusable for Sidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.list_focus.clone()
    }
}

/// The selection key of a row: the item's key, a folder's [`list::folder_key`].
fn row_key(items: &[ListItem], row: &Row) -> Option<String> {
    match row {
        Row::Item { index, .. } => Some(items[*index].key.clone()),
        Row::Folder { path, .. } => Some(list::folder_key(path)),
        _ => None,
    }
}

fn empty_text(section: Section, filtered: bool, running_only: bool, status: Option<&str>) -> String {
    match (section, status) {
        (Section::All, Some(s)) => s.to_string(),
        (Section::Pinned, _) => "nothing pinned".into(),
        (Section::All, None) if filtered => "no match".into(),
        (Section::All, None) if running_only => "nothing running".into(),
        (Section::All, None) => "empty".into(),
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let activity = self.activity();
        if activity == Activity::Terminals && self.state.visible && self.tick.is_none() {
            self.tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TERMINALS_TICK).await;
                    let shown = this.update(cx, |s, cx| {
                        let on = s.activity() == Activity::Terminals && s.state.visible;
                        if on {
                            cx.notify();
                        } else {
                            s.tick = None;
                        }
                        on
                    });
                    if !matches!(shown, Ok(true)) {
                        return;
                    }
                }
            }));
        }
        let count = self.rows(cx).1.len();
        let list_focused = self.list_focus.is_focused(window);
        let running_only = self.running_only;
        let filter = self.filter.read(cx).text().to_string();

        let header = div()
            .flex_none()
            .h(t.toolbar_h)
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().truncate().text_color(t.hi).child(activity.title()))
            .when(activity == Activity::Cluster, |d| {
                let protected = self.env(cx).is_some_and(|e| e.protected);
                d.child(label(format!("@{}", self.state.env.as_deref().unwrap_or("-"))).text_color(if protected { t.err } else { t.env }))
                    .child(
                        devdeck_ui::button("open-matrix", "matrix", &t)
                            .tooltip(tip("tags of every service on every env"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let view = ViewAddr::Matrix.to_ref();
                                this.deck.update(cx, |d, cx| {
                                    d.apply(cx, |l| {
                                        let p = l.main().focused;
                                        l.open(view, p).map(|_| ())
                                    });
                                });
                            })),
                    )
            })
            .when(activity == Activity::Apps, |d| {
                d.child(
                    toggle_button("running-only", "running", running_only, &t)
                        .tooltip(tip("show running only"))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_running_only(cx))),
                )
            });

        let list = uniform_list(
            "sidebar-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                let t = theme(cx).clone();
                let (items, rows) = this.rows(cx);
                let status = this.status(cx);
                let selected = this.selected_row(&items, &rows);
                let focused = this.list_focus.is_focused(window);
                let me = cx.entity();
                let look = Look { filter: filter.clone(), running_only: this.running_only(), focused, status, t };
                range
                    .filter_map(|i| rows.get(i).map(|r| (i, r.clone())))
                    .map(|(i, row)| {
                        let view_ref = match &row {
                            Row::Item { index, .. } => this.view_ref(&items[*index].key, cx),
                            _ => None,
                        };
                        render_row(&me, &this.deck, &items, &rows, i, row, selected == Some(i), &look, view_ref)
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();

        div()
            .id("sidebar")
            .key_context("Sidebar")
            .flex_none()
            .w(t.sidebar_w)
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.bar)
            .border_r_1()
            .border_color(t.line)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.select_edge(false, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.select_edge(true, cx)))
            .on_action(cx.listener(|this, _: &OpenSelected, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &ExpandSelected, _, cx| this.expand_selected(cx)))
            .on_action(cx.listener(|this, _: &CollapseSelected, _, cx| this.collapse_selected(cx)))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| this.toggle_pin_selected(cx)))
            .on_action(cx.listener(|this, _: &ToggleRunningOnly, _, cx| this.toggle_running_only(cx)))
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter.read(cx).focus_handle(cx).focus(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SidebarEscape, window, cx| this.escape(window, cx)))
            .child(header)
            .child(div().flex_none().px_2().pb_1().debug_selector(|| "sidebar-filter".into()).child(self.filter.clone()))
            .child(
                div()
                    .id("sidebar-list")
                    .track_focus(&self.list_focus)
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .when(list_focused, |d| d.border_t_1().border_color(t.focus))
                    .when(!list_focused, |d| d.border_t_1().border_color(t.line_soft))
                    .child(list),
            )

    }
}

/// What every row of one frame is drawn with.
struct Look {
    filter: String,
    running_only: bool,
    focused: bool,
    status: Option<String>,
    t: Theme,
}

#[allow(clippy::too_many_arguments)]
fn render_row(
    me: &Entity<Sidebar>,
    deck: &Entity<Deck>,
    items: &[ListItem],
    rows: &[Row],
    i: usize,
    row: Row,
    selected: bool,
    look: &Look,
    view_ref: Option<ViewRef>,
) -> gpui::AnyElement {
    let t = &look.t;
    let filtered = !look.filter.trim().is_empty();
    let sidebar_id = me.entity_id();
    let base = div().h(t.row_h).w_full().px_3().flex().items_center().gap_2().whitespace_nowrap();
    // Drops on a section (header, empty line or row) pin or unpin.
    let section_drop = |section: Section, el: gpui::Stateful<gpui::Div>| {
        let me = me.clone();
        let (bg, want) = (t.drop_bg, section == Section::Pinned);
        el.drag_over::<DraggedListItem>(move |s, d: &DraggedListItem, _, _| if d.sidebar == sidebar_id && d.pinned != want { s.bg(bg) } else { s })
            .on_drop(move |d: &DraggedListItem, _, cx| {
                if d.sidebar == sidebar_id && d.pinned != want {
                    let key = d.key.clone();
                    me.update(cx, |s, cx| s.toggle_pin(&key, cx));
                }
            })
    };
    // A tree row: selection look, hover, and the chevron opening or closing `path`.
    let tree = |key: &str, depth: usize, kids: Option<Kids>, path: Option<String>| {
        let m = me.clone();
        let hover = t.hover;
        tree_row(format!("row-{key}"), depth, kids.map(|k| k.open), move |_, cx| {
            if let Some(path) = &path {
                m.update(cx, |s, cx| s.toggle_open(path, cx));
            }
        }, t)
        .when(selected, |d| d.bg(if look.focused { t.selection } else { t.hover }))
        .hover(move |s| s.bg(hover))
        .cursor_pointer()
    };
    match row {
        Row::Header(section) => {
            let n = list::count(rows, section);
            let text = match section {
                Section::Pinned => format!("PINNED · {n}"),
                Section::All => format!("ALL · {n}"),
            };
            let selector = if section == Section::Pinned { "header-pinned" } else { "header-all" };
            section_drop(section, base.id(("hdr", i)).debug_selector(move || selector.into()).text_color(t.dim).text_size(t.text_small).child(text))
                .into_any_element()
        }
        Row::Empty(section) => section_drop(
            section,
            base.id(("empty", i)).pl_6().text_color(t.dim).child(label(empty_text(section, filtered, look.running_only, look.status.as_deref()))),
        )
        .into_any_element(),
        Row::Folder { path, name, depth, kids } => {
            let key = list::folder_key(&path);
            let m = me.clone();
            let el = tree(&key, depth, Some(kids), Some(path.clone()))
                .tooltip(tip(path.clone()))
                .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                    let double = e.click_count == 2;
                    m.update(cx, |s, cx| {
                        s.selected = Some(key.clone());
                        s.list_focus.focus(window, cx);
                        if double {
                            s.toggle_open(&path, cx);
                        }
                        cx.notify();
                    });
                })
                .child(div().flex_1().min_w_0().truncate().text_color(t.mute).child(format!("{name}/")))
                .when(!kids.open, |d| d.child(holds(kids, t)));
            section_drop(Section::All, el).into_any_element()
        }
        Row::Item { index, pinned, depth, kids } => {
            let item = &items[index];
            let section = if pinned { Section::Pinned } else { Section::All };
            let key = item.key.clone();
            let (k_click, k_menu) = (key.clone(), key.clone());
            let (m_click, m_menu) = (me.clone(), me.clone());
            let label_text: SharedString = item.label.clone().into();
            let drag = view_ref.map(|view| DraggedListItem { sidebar: sidebar_id, deck: deck.entity_id(), key: key.clone(), label: label_text.clone(), pinned, view });
            // Shown only to hold matches below it, or a library: de-emphasized.
            let context_only = !list::keeps(item, &look.filter, look.running_only);
            let text_color = if context_only { t.dim } else if item.warning.is_some() || item.dim { t.mute } else { t.fg };
            let el = tree(&key, depth, kids, rows[i].open_path(items).map(String::from))
                .tooltip(tip(label_text.clone()))
                .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                    let double = e.click_count == 2;
                    m_click.update(cx, |s, cx| {
                        s.selected = Some(k_click.clone());
                        s.list_focus.focus(window, cx);
                        if double {
                            s.open(&k_click, cx);
                        }
                        cx.notify();
                    });
                })
                .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                    m_menu.update(cx, |s, cx| {
                        s.selected = Some(k_menu.clone());
                        cx.notify();
                    });
                    let items = Sidebar::item_menu(&m_menu, &k_menu, pinned, cx);
                    open_menu(e.position, items, window, cx);
                })
                .when_some(drag, |el, drag| el.on_drag(drag, |d, _, _, cx| cx.new(|_| DragGhost(d.label.clone()))))
                .child(match &item.warning {
                    Some(_) => icon(Icon::Warning, t.warn, t).into_any_element(),
                    None => dot(if item.running { t.ok } else { t.dim }, item.running, t).into_any_element(),
                })
                .child(div().flex_1().min_w_0().truncate().text_color(text_color).child(label_text))
                .when_some(kids.filter(|k| !k.open), |d, k| d.child(holds(k, t)))
                .when(item.busy, |d| d.child(icon(Icon::Busy, t.accent, t)))
                .when_some(item.warning.clone().or((!item.detail.is_empty()).then(|| item.detail.clone())), |d, detail| {
                    let color = if item.warning.is_some() { t.warn } else { t.dim };
                    d.child(div().flex_none().max_w(gpui::relative(0.6)).truncate().text_color(color).child(detail))
                });
            section_drop(section, el).into_any_element()
        }
    }
}

/// What a closed row holds: how many, and whether something in it runs or can't run.
fn holds(kids: Kids, t: &Theme) -> gpui::Div {
    div()
        .flex_none()
        .flex()
        .gap_1()
        .text_size(t.text_small)
        .items_center()
        .when(kids.running, |d| d.child(dot(t.ok, true, t)))
        .when(kids.warning, |d| d.child(icon(Icon::Warning, t.warn, t)))
        .child(div().text_color(t.dim).child(kids.count.to_string()))
}
