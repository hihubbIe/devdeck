//! The sidebar of one workspace (R4): the activity bar's choice (Apps, Cluster, Ports)
//! and its list, Pinned then All, alphabetical, with a filter. Every workspace owns its
//! own `Sidebar`; the main window shows the active one.
//!
//! Selection is keyed by item, so it survives filtering and reordering; an item that
//! gets hidden drops the selection. Rows open their item as a tab in the main window's
//! focused pane (or wherever they are dropped), and pin or unpin with `p`, the menu, or a
//! drop on the other section.

use crate::deck::Deck;
use devdeck_core::demo::{Catalog, Env};
use devdeck_core::layout::{Drop, Layout, Result as LayoutResult, ViewRef};
use devdeck_core::list::{self, ListItem, Row, Section};
use devdeck_core::state::{SidebarState, StateStore};
use devdeck_ui::{DragGhost, InputEvent, MenuItem, TextInput, Theme, ToastKind, label, open_menu, theme, tip, toasts, toggle_button};
use gpui::{
    App, AppContext, Context, Entity, EntityId, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding,
    MouseButton, ParentElement, Render, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription,
    UniformListScrollHandle, Window, actions, div, prelude::*, uniform_list,
};

actions!(
    sidebar,
    [SelectNext, SelectPrev, SelectFirst, SelectLast, OpenSelected, TogglePin, FocusFilter, ToggleRunningOnly, SidebarEscape]
);

pub fn bind_keys(cx: &mut App) {
    let any = Some("Sidebar");
    let list = Some("Sidebar && !TextInput");
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, any),
        KeyBinding::new("up", SelectPrev, any),
        KeyBinding::new("home", SelectFirst, list),
        KeyBinding::new("end", SelectLast, list),
        KeyBinding::new("enter", OpenSelected, list),
        KeyBinding::new("p", TogglePin, list),
        KeyBinding::new("/", FocusFilter, list),
        KeyBinding::new("r", ToggleRunningOnly, list),
        KeyBinding::new("escape", SidebarEscape, any),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Apps,
    Cluster,
    Ports,
}

impl Activity {
    pub const ALL: [Activity; 3] = [Activity::Apps, Activity::Cluster, Activity::Ports];

    pub fn id(self) -> &'static str {
        match self {
            Activity::Apps => "apps",
            Activity::Cluster => "cluster",
            Activity::Ports => "ports",
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
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Activity::Apps => "Apps",
            Activity::Cluster => "Cluster",
            Activity::Ports => "Ports",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            Activity::Apps => "≡",
            Activity::Cluster => "◈",
            Activity::Ports => "⊡",
        }
    }

    pub fn key_hint(self) -> &'static str {
        match self {
            Activity::Apps => "ctrl 1",
            Activity::Cluster => "ctrl 2",
            Activity::Ports => "ctrl 3",
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
    catalog: Catalog,
    deck: Entity<Deck>,
    store: Option<StateStore>,
    pub state: SidebarState,
    pub running_only: bool,
    pub filter: Entity<TextInput>,
    pub list_focus: FocusHandle,
    pub selected: Option<String>,
    scroll: UniformListScrollHandle,
    save_failed: bool,
    _subs: Vec<Subscription>,
}

impl Sidebar {
    pub fn new(
        workspace: String,
        catalog: Catalog,
        deck: Entity<Deck>,
        store: Option<StateStore>,
        mut state: SidebarState,
        cx: &mut Context<Self>,
    ) -> Self {
        // Repair what the catalog can't honour (no cluster, unknown env or activity).
        let activity = Activity::from_id(&state.activity).unwrap_or(Activity::Apps);
        let activity = if activity == Activity::Cluster && catalog.cluster.is_none() { Activity::Apps } else { activity };
        state.activity = activity.id().into();
        state.env = catalog.cluster.as_ref().map(|c| match &state.env {
            Some(e) if c.envs.iter().any(|x| &x.name == e) => e.clone(),
            _ => c.default_env.clone(),
        });
        let filter = cx.new(|cx| TextInput::new("filter", cx));
        let subs = vec![cx.subscribe(&filter, |this, _, e: &InputEvent, cx| match e {
            InputEvent::Changed(_) => {
                this.drop_hidden_selection(cx);
                cx.notify();
            }
            InputEvent::Submit(_) => this.open_selected(cx),
            InputEvent::Cancel => {}
        })];
        Self {
            workspace,
            catalog,
            deck,
            store,
            state,
            running_only: false,
            filter,
            list_focus: cx.focus_handle(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            save_failed: false,
            _subs: subs,
        }
    }

    pub fn activity(&self) -> Activity {
        Activity::from_id(&self.state.activity).unwrap_or(Activity::Apps)
    }

    /// Activities this workspace offers (Cluster only with a cluster, R2.3).
    pub fn activities(&self) -> Vec<Activity> {
        Activity::ALL.into_iter().filter(|a| *a != Activity::Cluster || self.catalog.cluster.is_some()).collect()
    }

    pub fn envs(&self) -> &[Env] {
        self.catalog.cluster.as_ref().map_or(&[], |c| &c.envs)
    }

    pub fn env(&self) -> Option<&Env> {
        let name = self.state.env.as_deref()?;
        self.envs().iter().find(|e| e.name == name)
    }

    pub fn items(&self) -> Vec<ListItem> {
        match self.activity() {
            Activity::Apps => self.catalog.apps.clone(),
            Activity::Cluster => match (&self.catalog.cluster, &self.state.env) {
                (Some(c), Some(env)) => c.deployments(env),
                _ => Vec::new(),
            },
            Activity::Ports => self.catalog.ports.clone(),
        }
    }

    fn pins(&self) -> std::collections::BTreeSet<String> {
        self.state.pins.get(self.activity().list_id()).cloned().unwrap_or_default()
    }

    pub fn rows(&self, cx: &App) -> (Vec<ListItem>, Vec<Row>) {
        let items = self.items();
        let running_only = self.running_only && self.activity() == Activity::Apps;
        let rows = list::rows(&items, &self.pins(), self.filter.read(cx).text(), running_only);
        (items, rows)
    }

    fn selected_row(&self, items: &[ListItem], rows: &[Row]) -> Option<usize> {
        let key = self.selected.as_deref()?;
        rows.iter().position(|r| matches!(r, Row::Item { index, .. } if items[*index].key == key))
    }

    fn drop_hidden_selection(&mut self, cx: &App) {
        let (items, rows) = self.rows(cx);
        if self.selected_row(&items, &rows).is_none() {
            self.selected = None;
        }
    }

    fn select_row(&mut self, items: &[ListItem], rows: &[Row], row: Option<usize>, cx: &mut Context<Self>) {
        if let Some(r) = row
            && let Row::Item { index, .. } = rows[r]
        {
            self.selected = Some(items[index].key.clone());
            self.scroll.scroll_to_item(r, ScrollStrategy::Nearest);
        }
        cx.notify();
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

    /// The view address an item opens as.
    pub fn view_ref(&self, key: &str) -> ViewRef {
        match self.activity() {
            Activity::Apps => ViewRef::new(format!("app/{key}")),
            Activity::Cluster => ViewRef::new(format!("deploy/{}/{key}", self.state.env.as_deref().unwrap_or("-"))),
            Activity::Ports => ViewRef::new(format!("port/{key}")),
        }
    }

    /// Opens an item in the main window's focused pane; an open one is activated.
    pub fn open(&mut self, key: &str, cx: &mut Context<Self>) {
        let view = self.view_ref(key);
        self.deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let p = l.main().focused;
                l.open(view, p).map(|_| ())
            });
        });
    }

    pub fn open_selected(&mut self, cx: &mut Context<Self>) {
        let (items, rows) = self.rows(cx);
        if self.selected_row(&items, &rows).is_none() {
            // Nothing selected (e.g. right after typing a filter): take the first match.
            let first = list::step(&rows, None, 0);
            self.select_row(&items, &rows, first, cx);
        }
        if let Some(key) = self.selected.clone() {
            self.open(&key, cx);
        }
    }

    pub fn toggle_pin(&mut self, key: &str, cx: &mut Context<Self>) {
        let pins = self.state.pins.entry(self.activity().list_id().to_string()).or_default();
        list::toggle_pin(pins, key);
        self.save(cx);
        cx.notify();
    }

    fn toggle_pin_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.selected.clone() {
            self.toggle_pin(&key, cx);
            let (items, rows) = self.rows(cx);
            let row = self.selected_row(&items, &rows);
            self.select_row(&items, &rows, row, cx);
        }
    }

    /// Shows an activity (and the sidebar); unknown or unavailable ones are ignored.
    /// Returns whether it is now shown.
    pub fn show(&mut self, activity: Activity, cx: &mut Context<Self>) -> bool {
        if !self.activities().contains(&activity) {
            return false;
        }
        if self.activity() != activity {
            self.state.activity = activity.id().into();
            self.selected = None;
            self.filter.update(cx, |f, cx| f.set_text("", cx));
        }
        self.state.visible = true;
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
        if self.envs().iter().any(|e| e.name == env) && self.state.env.as_deref() != Some(env) {
            self.state.env = Some(env.into());
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

    pub fn item_menu(this: &Entity<Self>, key: &str, pinned: bool) -> Vec<MenuItem> {
        let (open, pin) = (this.clone(), this.clone());
        let (k1, k2) = (key.to_string(), key.to_string());
        vec![
            MenuItem::new("Open", move |_, cx| open.update(cx, |s, cx| s.open(&k1, cx))).hint("⏎"),
            MenuItem::new(if pinned { "Unpin" } else { "Pin" }, move |_, cx| pin.update(cx, |s, cx| s.toggle_pin(&k2, cx))).hint("p"),
        ]
    }

    pub fn env_menu(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let s = this.read(cx);
        let current = s.state.env.clone();
        s.envs()
            .iter()
            .map(|e| {
                let (sidebar, name) = (this.clone(), e.name.clone());
                let label = if e.protected { format!("⊘ {}", e.name) } else { e.name.clone() };
                MenuItem::new(label, move |_, cx| sidebar.update(cx, |s, cx| s.set_env(&name, cx))).checked(current.as_deref() == Some(&e.name))
            })
            .collect()
    }
}

impl Focusable for Sidebar {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.list_focus.clone()
    }
}

fn empty_text(section: Section, filtered: bool, running_only: bool) -> &'static str {
    match section {
        Section::Pinned => "nothing pinned · p pins",
        Section::All if filtered => "no match",
        Section::All if running_only => "nothing running",
        Section::All => "empty",
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let activity = self.activity();
        let (items, rows) = self.rows(cx);
        let count = rows.len();
        let list_focused = self.list_focus.is_focused(window);
        let filtered = !self.filter.read(cx).text().trim().is_empty();
        let running_only = self.running_only;

        let header = div()
            .flex_none()
            .h(t.toolbar_h)
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().truncate().text_color(t.hi).child(activity.title()))
            .when(activity == Activity::Cluster, |d| {
                let protected = self.env().is_some_and(|e| e.protected);
                d.child(label(format!("@{}", self.state.env.as_deref().unwrap_or("-"))).text_color(if protected { t.err } else { t.env }))
            })
            .when(activity == Activity::Apps, |d| {
                d.child(
                    toggle_button("running-only", "running", running_only, &t)
                        .tooltip(tip("show running only (r)"))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_running_only(cx))),
                )
            });

        let list = uniform_list(
            "sidebar-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                let t = theme(cx).clone();
                let (items, rows) = this.rows(cx);
                let selected = this.selected_row(&items, &rows);
                let focused = this.list_focus.is_focused(window);
                let me = cx.entity();
                range
                    .filter_map(|i| rows.get(i).map(|r| (i, r.clone())))
                    .map(|(i, row)| render_row(&me, &this.deck, &items, &rows, i, row, selected == Some(i), focused, filtered, running_only, &t, this))
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
            .child(
                div()
                    .flex_none()
                    .h(t.status_h)
                    .px_3()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(t.line_soft)
                    .text_color(t.dim)
                    .child(label(format!("{} items · ↑↓ ⏎ open · p pin · / filter", items.len()))),
            )
    }
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
    focused: bool,
    filtered: bool,
    running_only: bool,
    t: &Theme,
    sidebar: &Sidebar,
) -> gpui::AnyElement {
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
    match row {
        Row::Header(section) => {
            let n = sidebar_count(rows, section);
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
            base.id(("empty", i)).pl_6().text_color(t.dim).child(label(empty_text(section, filtered, running_only))),
        )
        .into_any_element(),
        Row::Item { index, pinned } => {
            let item = &items[index];
            let section = if pinned { Section::Pinned } else { Section::All };
            let key = item.key.clone();
            let (k_click, k_menu) = (key.clone(), key.clone());
            let (m_click, m_menu) = (me.clone(), me.clone());
            let label_text: SharedString = item.label.clone().into();
            let drag = DraggedListItem {
                sidebar: sidebar_id,
                deck: deck.entity_id(),
                key: key.clone(),
                label: label_text.clone(),
                pinned,
                view: sidebar.view_ref(&key),
            };
            let hover = t.hover;
            let selector = format!("row-{key}");
            let el = base
                .id(("row", i))
                .debug_selector(move || selector.clone())
                .when(selected, |d| d.bg(if focused { t.selection } else { t.hover }))
                .hover(move |s| s.bg(hover))
                .cursor_pointer()
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
                    let items = Sidebar::item_menu(&m_menu, &k_menu, pinned);
                    open_menu(e.position, items, window, cx);
                })
                .on_drag(drag, |d, _, _, cx| cx.new(|_| DragGhost(d.label.clone())))
                .child(
                    div()
                        .flex_none()
                        .text_color(if item.running { t.ok } else { t.dim })
                        .child(if item.running { "●" } else { "○" }),
                )
                .child(div().flex_1().min_w_0().truncate().text_color(t.fg).child(label_text))
                .when(!item.detail.is_empty(), |d| {
                    d.child(div().flex_none().max_w(gpui::relative(0.6)).truncate().text_color(t.dim).child(item.detail.clone()))
                });
            section_drop(section, el).into_any_element()
        }
    }
}

/// Items shown in a section (header counts follow the filter).
fn sidebar_count(rows: &[Row], section: Section) -> usize {
    rows.iter().filter(|r| matches!(r, Row::Item { pinned, .. } if *pinned == (section == Section::Pinned))).count()
}
