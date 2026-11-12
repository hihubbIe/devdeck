//! The sidebar of one workspace (R4): the activity bar's choice (Apps, Cluster, Ports,
//! Layouts, Terminals, Notes) and its list, Pinned then All, alphabetical, with a filter.
//! Every workspace owns its own `Sidebar`; the main window shows the active one.
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
//!
//! Notes (R12.3) are a tree too: one level of projects holding their notes, then the
//! notes without a project; projects without notes don't show.

use crate::deck::Deck;
use crate::open_in::{self, OpenedFrom};
use crate::runner::RunKey;
use crate::services::Services;
use devdeck_core::cluster::{Env, Remote};
use devdeck_core::view_addr::ViewAddr;
use devdeck_core::layout::{Drop, Layout, Result as LayoutResult, ViewRef};
use devdeck_core::list::{self, Groups, Kids, ListItem, Row, Section, Shape, group_of_row, group_row_path};
use devdeck_core::state::{SidebarState, StateStore};
use devdeck_ui::scroll_list;
use devdeck_ui::{DragGhost, Icon, InputEvent, MenuItem, TextInput, Theme, ToastKind, dot, icon, icon_text, label, open_menu, theme, tip, toasts, toggle_button, tree_row};
use gpui::{
    App, AppContext, Context, Entity, EntityId, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription,
    UniformListScrollHandle, Window, actions, div, prelude::*,
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
        ToggleWatchPods,
        OpenListEnvMenu,
        NewNote,
        SidebarEscape
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Activity {
    Apps,
    Cluster,
    Ports,
    /// The saved layouts, "pulses" to the user.
    Layouts,
    /// Every shell of the workspace, shown or not.
    Terminals,
    /// The workspace's notes, by project.
    Notes,
    /// The workspace's agent sessions (R6.14).
    Agents,
}

impl Activity {
    /// In the activity bar's order; the first is where a workspace lands.
    pub const ALL: [Activity; 7] = [Activity::Layouts, Activity::Apps, Activity::Cluster, Activity::Ports, Activity::Terminals, Activity::Notes, Activity::Agents];

    pub fn id(self) -> &'static str {
        match self {
            Activity::Apps => "apps",
            Activity::Cluster => "cluster",
            Activity::Ports => "ports",
            Activity::Layouts => "pulses",
            Activity::Terminals => "terminals",
            Activity::Notes => "notes",
            Activity::Agents => "agents",
        }
    }

    /// Whether it shows as a full view: its list, then its selection's details instead
    /// of the pulse's panes (Pulses: its list beside the panes).
    /// Terminals, Notes and Agents show the terminal, the note or the session itself.
    pub fn full_view(self) -> bool {
        matches!(self, Activity::Apps | Activity::Cluster | Activity::Ports | Activity::Terminals | Activity::Notes | Activity::Agents)
    }

    /// What a full view's details say while nothing is selected.
    pub fn pick(self) -> &'static str {
        match self {
            Activity::Cluster => "pick a service",
            Activity::Terminals => "pick a terminal",
            Activity::Notes => "pick a note",
            Activity::Ports => "pick a port",
            Activity::Agents => "pick a session",
            _ => "pick an app",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    /// The pin set it uses. Deployment pins are shared by every env.
    pub fn list_id(self) -> &'static str {
        match self {
            Activity::Apps => "apps",
            Activity::Cluster => "deployments",
            Activity::Ports => "ports",
            Activity::Layouts => "layouts",
            Activity::Terminals => "terminals",
            Activity::Notes => "notes",
            Activity::Agents => "agents",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Activity::Apps => "Apps",
            Activity::Cluster => "Cluster",
            Activity::Ports => "Ports",
            Activity::Layouts => "Pulses",
            Activity::Terminals => "Terminals",
            Activity::Notes => "Notes",
            Activity::Agents => "Agents",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Activity::Apps => Icon::Apps,
            Activity::Cluster => Icon::Cluster,
            Activity::Ports => Icon::Ports,
            Activity::Layouts => Icon::Layouts,
            Activity::Terminals => Icon::Terminals,
            Activity::Notes => Icon::Note,
            Activity::Agents => Icon::Agents,
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

/// A folder or group row being dragged: onto a group to go in it, onto All to go back
/// to the top.
#[derive(Clone)]
pub struct DraggedFolder {
    pub sidebar: EntityId,
    /// A folder's real path, or a group's [`group_row_path`].
    pub path: String,
    pub label: SharedString,
}

/// Where a drop lands: the Pinned section, a row of All in a group (`None`: at the
/// top), or the All header (the top, a pinned item included).
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Pinned,
    All(Option<String>),
    AllHeader,
}

/// What a drop does.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Pins or unpins the item (unpinned, it goes back to its group).
    TogglePin(String),
    /// Puts a node in a group or at the top (unpinning it).
    Put(String, Option<String>),
    MoveGroup(String, Option<String>),
}

/// What a group's menu or a prompt moves into a group.
#[derive(Clone, Debug)]
pub enum Moving {
    /// An item's key (its node path) or a folder's path.
    Node(String),
    /// A group's path.
    Group(String),
}

/// Opens `view` at `drop` (into a pane or beside something); an open view moves there.
pub fn open_at(l: &mut Layout, view: ViewRef, drop: Drop) -> LayoutResult<()> {
    l.open_at(view, drop)
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
    /// Each list's selection while another shows: back to it, it's where it was.
    left: std::collections::HashMap<Activity, String>,
    /// Cluster: the branch matrix the whole width, the list hidden.
    pub matrix: bool,
    scroll: UniformListScrollHandle,
    save_failed: bool,
    /// The list env's pods, while the Cluster list shows with **watch** on.
    pods: Option<(String, crate::cluster::PodsInterest)>,
    /// While the Ports list shows: its ports scanned (the known ones watched), and the
    /// whole machine with "machine" on.
    ports_watch: Option<(crate::port_scan::PortsInterest, Option<crate::port_scan::MachineInterest>)>,
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
        // Repair what the workspace can't honour (no cluster, unknown activity, such as
        // "layouts" saved before they were named pulses): the first activity, pulses. The
        // services already settled the env.
        // It opens on Pulses whatever was shown last (user, 2026-11-11): where a workspace
        // lands. Each list keeps what it had selected.
        state.activity = Activity::ALL[0].id().into();
        state.env = services.read(cx).env.clone();
        // An env the catalog no longer has: back to following the workspace's.
        if state.cluster_env.as_ref().is_some_and(|e| services.read(cx).catalog.env(e).is_none()) {
            state.cluster_env = None;
        }
        let filter = cx.new(|cx| TextInput::new("filter", cx));
        let notes = services.read(cx).notes.clone();
        let subs = vec![
            // The Layouts list reads the deck's book.
            cx.observe(&deck, |_, _, cx| cx.notify()),
            cx.observe(&notes, |this, _, cx| {
                this.drop_hidden_selection(cx);
                cx.notify();
            }),
            cx.observe(&services, |this, _, cx| {
                this.drop_hidden_selection(cx);
                // The cluster may have come back, or the env changed under the list.
                this.load_list(cx);
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
        let mut this = Self {
            workspace,
            services,
            deck,
            store,
            state,
            running_only: false,
            filter,
            list_focus: cx.focus_handle(),
            selected: None,
            left: Default::default(),
            matrix: false,
            scroll: UniformListScrollHandle::new(),
            save_failed: false,
            pods: None,
            ports_watch: None,
            tick: None,
            _subs: subs,
        };
        this._subs.push(cx.observe_self(|this, cx| this.follow(cx)));
        this.load_list(cx);
        this.follow(cx);
        this
    }

    /// On a full view the details show its selection (none for a folder); elsewhere the
    /// pulse's panes show.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let base = self.activity().full_view().then(|| match self.activity() {
            Activity::Cluster if self.matrix => Some(ViewAddr::Matrix.to_ref()),
            // Nothing selected: the overview of every session.
            Activity::Agents if self.selected.is_none() => Some(ViewAddr::Agents.to_ref()),
            _ => self.selected.as_deref().filter(|k| !k.ends_with('/')).and_then(|k| self.view_ref(k, cx)),
        });
        self.deck.update(cx, |d, cx| match base {
            Some(base) => d.show_outside(base, cx),
            None => d.hide_outside(cx),
        });
    }

    /// Starts loading what the shown list needs: the env's deployments, and with **watch**
    /// on and the list on screen, its pods (a new handle before the old one goes, so a same-env refresh keeps the
    /// watch).
    fn load_list(&mut self, cx: &mut Context<Self>) {
        self.watch_ports(cx);
        let list_env = self.list_env(cx);
        let s = self.services.read(cx);
        let target = match (self.activity(), s.cluster.clone(), list_env) {
            (Activity::Cluster, Some(cluster), Some(env)) => Some((cluster, env)),
            _ => None,
        };
        let Some((cluster, env)) = target else {
            self.pods = None;
            return;
        };
        // The pods only while the list is on screen.
        let watch = self.state.watch_pods && self.state.visible;
        let pods = cluster.update(cx, |c, cx| {
            c.want(&env, cx);
            watch.then(|| c.pods_interest(&env, crate::cluster::PodScope::Env, cx))
        });
        self.pods = pods.map(|p| (env, p));
    }

    /// **watch**: the Cluster list watches every pod of its env, and marks deployments in
    /// trouble; off, only open services' pods are watched.
    pub fn toggle_watch_pods(&mut self, cx: &mut Context<Self>) {
        if self.activity() == Activity::Cluster {
            self.state.watch_pods = !self.state.watch_pods;
            self.load_list(cx);
            self.save(cx);
            cx.notify();
        }
    }

    /// "machine": the Ports list shows every listener of the machine, or the workspace's.
    pub fn toggle_ports_machine(&mut self, cx: &mut Context<Self>) {
        self.state.ports_machine = !self.state.ports_machine;
        self.watch_ports(cx);
        self.save(cx);
        cx.notify();
    }

    /// Scans while the Ports list shows: the ports the workspace knows watched (who holds
    /// them), the whole machine with "machine" on.
    fn watch_ports(&mut self, cx: &mut Context<Self>) {
        let shown = self.activity() == Activity::Ports && (self.activity().full_view() || self.state.visible);
        if !shown {
            self.ports_watch = None;
            return;
        }
        let s = self.services.read(cx);
        let (ports, machine) = (s.ports.clone(), self.state.ports_machine);
        let declared: std::collections::BTreeSet<u16> = s.catalog.apps.iter().flat_map(|a| a.commands.iter().flat_map(|c| c.ports.iter().copied())).collect();
        self.ports_watch = Some(ports.update(cx, |p, cx| {
            let known: std::collections::BTreeSet<u16> = declared.into_iter().chain(p.known().values().flatten().copied()).collect();
            (p.interest(known, cx), machine.then(|| p.machine(cx)))
        }));
    }

    pub fn watching_pods(&self) -> bool {
        self.pods.is_some()
    }

    pub fn activity(&self) -> Activity {
        Activity::from_id(&self.state.activity).unwrap_or(Activity::ALL[0])
    }

    /// Activities this workspace offers (Cluster only with a cluster, R2.3).
    pub fn activities(&self, cx: &App) -> Vec<Activity> {
        let has_cluster = self.services.read(cx).cluster.is_some();
        Activity::ALL.into_iter().filter(|a| *a != Activity::Cluster || has_cluster).collect()
    }

    pub fn envs(&self, cx: &App) -> Vec<Env> {
        self.services.read(cx).catalog.envs.clone()
    }

    /// The workspace's env (runs, the title bar).
    pub fn env(&self, cx: &App) -> Option<Env> {
        let name = self.services.read(cx).env.clone()?;
        self.envs(cx).into_iter().find(|e| e.name == name)
    }

    /// The env the Cluster list shows: its own when one is picked, else the workspace's.
    pub fn list_env(&self, cx: &App) -> Option<String> {
        self.state.cluster_env.clone().filter(|e| self.envs(cx).iter().any(|x| &x.name == e)).or_else(|| self.services.read(cx).env.clone())
    }

    /// Whether the Cluster list follows the workspace's env.
    pub fn list_synced(&self) -> bool {
        self.state.cluster_env.is_none()
    }

    /// The Cluster list shows `env` (the workspace's stays), or follows the workspace's
    /// again (`None`).
    pub fn set_list_env(&mut self, env: Option<&str>, cx: &mut Context<Self>) {
        // An env the workspace doesn't have changes nothing.
        if env.is_some_and(|e| !self.envs(cx).iter().any(|x| x.name == e)) {
            return;
        }
        let env = env.map(str::to_string);
        if env == self.state.cluster_env {
            return;
        }
        self.state.cluster_env = env;
        self.load_list(cx);
        self.drop_hidden_selection(cx);
        self.save(cx);
        cx.notify();
    }

    /// The Cluster list's env menu by key, under the list's header.
    pub fn open_list_env_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.activity() != Activity::Cluster || self.envs(cx).is_empty() {
            return;
        }
        let t = theme(cx);
        let at = gpui::point(t.size(160.), t.title_h + t.toolbar_h);
        // The menu reads the sidebar: once this update is over.
        let me = cx.entity();
        window.defer(cx, move |window, cx| open_menu(at, Sidebar::list_env_menu(&me, cx), window, cx));
    }

    /// The Cluster list's env menu: following the workspace's, or one of the envs.
    pub fn list_env_menu(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let s = this.read(cx);
        let err = theme(cx).err;
        let workspace = s.services.read(cx).env.clone().unwrap_or_else(|| "-".into());
        let sync = {
            let sidebar = this.clone();
            MenuItem::new(format!("sync with workspace (@{workspace})"), move |_, cx| sidebar.update(cx, |s, cx| s.set_list_env(None, cx))).checked(s.list_synced())
        };
        let own = s.state.cluster_env.clone();
        std::iter::once(sync)
            .chain(s.envs(cx).iter().map(|e| {
                let (sidebar, name) = (this.clone(), e.name.clone());
                let item = MenuItem::new(e.name.clone(), move |_, cx| sidebar.update(cx, |s, cx| s.set_list_env(Some(&name), cx))).checked(own.as_deref() == Some(&e.name));
                if e.protected { item.icon(Icon::Protected, err) } else { item }
            }))
            .collect()
    }

    /// What's wrong with the cluster list's env while its last list stays shown (a lost
    /// connection).
    pub fn kept_problem(&self, cx: &App) -> Option<String> {
        if self.activity() != Activity::Cluster {
            return None;
        }
        let env = self.list_env(cx)?;
        let s = self.services.read(cx);
        let (c, env) = (s.cluster.as_ref()?.read(cx), env.as_str());
        let deployments = matches!(c.peek(env), Remote::Ready(_)).then(|| c.problem(env).map(str::to_string)).flatten();
        let pods = self.state.watch_pods.then(|| c.pods_problem(env).map(|p| format!("pods: {p}"))).flatten();
        deployments.or(pods)
    }

    /// Why the list is empty when it is for a reason (loading, unreachable env).
    pub fn status(&self, cx: &App) -> Option<String> {
        if self.activity() != Activity::Cluster {
            return None;
        }
        let env = self.list_env(cx);
        let s = self.services.read(cx);
        match (s.cluster.as_ref(), env.as_deref()) {
            (Some(c), Some(env)) => match c.read(cx).peek(env) {
                Remote::Loading => Some("loading…".into()),
                Remote::Error(e) => Some(e),
                Remote::Ready(_) => None,
            },
            _ => None,
        }
    }

    pub fn items(&self, cx: &App) -> Vec<ListItem> {
        self.items_in(self.activity(), cx)
    }

    /// Whether `list` shows `node`: an item (its key or its path), or a folder of its tree.
    pub fn knows(&self, list: Activity, node: &str, cx: &App) -> bool {
        let items = self.project_tree(list, cx).map_or_else(|| self.items_in(list, cx), |t| t.items);
        let folder = format!("{node}/");
        items.iter().any(|i| i.key == node || i.path.as_ref().is_some_and(|p| p == node || p.starts_with(&folder)))
    }

    /// The items of `list`, shown or not.
    pub fn items_in(&self, list: Activity, cx: &App) -> Vec<ListItem> {
        let list_env = self.list_env(cx);
        let s = self.services.read(cx);
        match list {
            Activity::Apps => {
                let runner = s.runner.read(cx);
                s.catalog
                    .apps
                    .iter()
                    .map(|a| {
                        // No right text (the toolchain, the reason): a missing tool is ⚠,
                        // its reason on hover with the name.
                        let warning = s.unusable.get(&a.id).cloned();
                        ListItem::new(a.id.clone(), a.name.clone())
                            .hint(warning.as_ref().map(|w| format!("{} · {w}", a.name)))
                            .running(runner.app_running(&a.id))
                            .warning(warning)
                            .path(a.id.clone())
                            .dim(a.library)
                    })
                    .collect()
            }
            Activity::Cluster => match (s.cluster.as_ref(), list_env.as_deref()) {
                (Some(c), Some(env)) => {
                    let c = c.read(cx);
                    let watch = self.state.watch_pods;
                    let now = c.now_s();
                    c.peek(env)
                        .ready()
                        .map(|deps| {
                            deps.values()
                                .map(|d| {
                                    let rollout = c.rollout(env, &d.name).filter(|r| !r.finished());
                                    let rolling = rollout.is_some_and(|r| matches!(r.state, devdeck_core::cluster::RolloutState::Rolling { .. }));
                                    let failing = rollout.and_then(|r| match &r.state {
                                        devdeck_core::cluster::RolloutState::Failing { reason, .. } | devdeck_core::cluster::RolloutState::Stuck { reason } => Some(reason.clone()),
                                        _ => None,
                                    });
                                    // The other containers and, watching pods, running since and restarts, on hover.
                                    let others: Vec<String> = d.containers.iter().filter(|x| x.name != d.main().name).map(|x| format!("{}: {}", x.name, x.image.label())).collect();
                                    let mut hint: Vec<String> = Vec::new();
                                    if !others.is_empty() {
                                        hint.push(format!("also {}", others.join(", ")));
                                    }
                                    let pods = if watch { c.pods(env, &d.name) } else { &[] };
                                    if watch {
                                        if let Some(since) = c.since(env, d) {
                                            hint.push(format!("since {}", crate::views::fmt_age(now.saturating_sub(since))));
                                        }
                                        hint.push(format!("{} restarts", pods.iter().map(|p| p.restarts).sum::<u32>()));
                                    }
                                    let hint = (!hint.is_empty()).then(|| format!("{} {} · {}", d.name, d.tag(), hint.join(" · ")));
                                    // Watching pods: a pod in trouble marks the row with its reason.
                                    let trouble = failing.or_else(|| pods.iter().find_map(|p| p.trouble()).map(str::to_string));
                                    ListItem::new(d.name.clone(), d.name.clone())
                                        .busy(rolling)
                                        .running(d.ready == d.replicas)
                                        .hint(hint)
                                        .pods(d.ready, d.replicas, trouble.is_some())
                                        .warning(trouble)
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                }
                _ => Vec::new(),
            },
            // Held now (the holder), or known and free (the command that wants it).
            Activity::Ports => {
                let runner = s.runner.read(cx);
                s.port_rows(self.state.ports_machine, cx)
                    .into_iter()
                    .map(|r| {
                        let detail = match (&r.held, r.wants.first()) {
                            (Some(h), _) => h.holder.clone(),
                            (None, Some(w)) => runner.title(&w.key),
                            (None, None) => String::new(),
                        };
                        let state = if r.held.is_none() { "free" } else if r.foreign { "not devdeck's" } else { "listening" };
                        let hint = [format!(":{}", r.port), detail.clone(), state.to_string()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                        ListItem::new(r.port.to_string(), format!(":{}", r.port)).detail(detail).running(r.held.is_some()).hint(Some(hint))
                    })
                    .collect()
            }
            Activity::Layouts => {
                let book = &self.deck.read(cx).book;
                book.entries()
                    .into_iter()
                    .map(|(id, name)| {
                        let current = id == book.current;
                        ListItem::new(id, name).detail(if current { "current" } else { "" }).running(current)
                    })
                    .collect()
            }
            Activity::Terminals => {
                // Every terminal (R6.12): running or not, shown or not (its shell outlives
                // its tabs), what it does and where it shows.
                let runner = s.runner.read(cx);
                self.deck
                    .read(cx)
                    .terminals(cx)
                    .into_iter()
                    .map(|(n, tabs)| {
                        let key = RunKey::terminal(n);
                        let state = runner.state(&key);
                        let doing = match (runner.terminal_program(&key), state) {
                            (Some(program), _) => program,
                            (None, Some(s)) if s.is_running() => "idle".into(),
                            (None, Some(_)) => "ended".into(),
                            (None, None) => "not started".into(),
                        };
                        let shows = match (tabs.here, tabs.layouts.as_slice()) {
                            (true, _) => "here".to_string(),
                            (false, []) => "not shown".to_string(),
                            (false, l) => format!("in {}", l.iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>().join(", ")),
                        };
                        let dir = match ViewAddr::parse(&tabs.view) {
                            ViewAddr::Terminal { dir, .. } => dir,
                            _ => String::new(),
                        };
                        ListItem::new(n.to_string(), crate::views::terminal_name(&dir))
                            .detail(format!("{doing} · {shows}"))
                            .hint(Some(format!("{} · {doing} · {shows}", devdeck_core::config::shown(std::path::Path::new(&dir)))))
                            .running(state.is_some_and(|s| s.is_running()))
                    })
                    .collect()
            }
            Activity::Notes => self.notes_tree(cx).items,
            Activity::Agents => self.agents_tree(cx).items,
        }
    }

    /// A list shown as a tree of projects (Notes, Agents): its items under their project's
    /// node, with each node's name, hint, and whether its app went away.
    pub fn project_tree(&self, list: Activity, cx: &App) -> Option<devdeck_core::notes::Tree> {
        match list {
            Activity::Notes => Some(self.notes_tree(cx)),
            Activity::Agents => Some(self.agents_tree(cx)),
            _ => None,
        }
    }

    /// The Agents list (R6.14): every session, running or not, under its project (named by
    /// its app, dim once gone); a session in no app's folder at the top.
    pub fn agents_tree(&self, cx: &App) -> devdeck_core::notes::Tree {
        use devdeck_core::agents::Status;
        let s = self.services.read(cx);
        let agents = s.agents.read(cx);
        let mut tree = devdeck_core::notes::Tree::default();
        for a in agents.sessions() {
            let status = agents.status(&a.id, cx);
            let label = agents.label(&a.id, cx);
            let item = ListItem::new(a.id.clone(), label.clone())
                .detail(status.label())
                .running(status != Status::Off)
                .busy(status == Status::Working)
                .warning((status == Status::NeedsYou).then(|| "needs you".to_string()));
            let item = match &a.project {
                Some(p) => {
                    let node = devdeck_core::notes::project_node(p);
                    if !tree.names.contains_key(&node) {
                        let (name, found) = agents.project_name(p);
                        if !found {
                            tree.gone.insert(node.clone());
                        }
                        tree.hints.insert(node.clone(), if found { p.clone() } else { format!("{p} · project not found") });
                        tree.names.insert(node.clone(), name);
                    }
                    let project = &tree.names[&node];
                    item.hint(Some(format!("{project} · {label} · {}", status.label()))).path(format!("{node}/{}", a.id))
                }
                None => item.hint(Some(format!("{label} · {} · {}", status.label(), devdeck_core::config::shown(&a.dir)))),
            };
            tree.items.push(item);
        }
        tree
    }

    /// The Notes list: notes by project (named by its app, or its id once gone).
    pub fn notes_tree(&self, cx: &App) -> devdeck_core::notes::Tree {
        let s = self.services.read(cx);
        devdeck_core::notes::tree(s.notes.read(cx).all(), |app| s.catalog.app(app).map(|a| a.name.clone()))
    }

    /// Asks a name and makes a note for `app` (empty: no project), opened in this deck.
    pub fn new_note(this: &Entity<Self>, app: &str, window: &mut Window, cx: &mut App) {
        let s = this.read(cx);
        let notes = s.services.read(cx).notes.clone();
        // Made from the Notes list: selected there, so its full view shows it.
        let me = this.downgrade();
        let open: crate::views::note_view::Opener = std::rc::Rc::new(move |addr, cx| {
            if let ViewAddr::Note { id } = addr {
                let _ = me.update(cx, |s, cx| s.show_item(Activity::Notes, &id, cx));
            }
        });
        crate::views::note_view::ask_note(notes, app, open, window, cx);
    }

    /// A project row's menu in the Agents list: a shell or a session in its folder.
    fn agents_project_menu(this: &Entity<Self>, node: &str, cx: &App) -> Vec<MenuItem> {
        let s = this.read(cx);
        let app = devdeck_core::notes::project_of_node(node);
        let Some(dir) = s.services.read(cx).catalog.app(&app).map(|a| a.dir.clone()) else { return Vec::new() };
        open_in::item(&s.deck, open_in::Subject::folder(dir), OpenedFrom::Outside, cx).into_iter().collect()
    }

    /// "Open in" for a row's view (`None`: nothing to open), after a separator when the
    /// menu has items before it.
    fn open_in(this: &Entity<Self>, view: Option<ViewRef>, items: &mut Vec<MenuItem>, cx: &App) {
        let Some(view) = view else { return };
        let deck = this.read(cx).deck.clone();
        let dir = deck.read(cx).own_folder(&view, cx);
        if let Some(item) = open_in::item(&deck, open_in::Subject::view(view, dir), OpenedFrom::Outside, cx) {
            if !items.is_empty() {
                items.push(MenuItem::separator());
            }
            items.push(item);
        }
    }

    /// A project row's menu in the Notes list.
    fn project_menu(this: &Entity<Self>, node: &str) -> Vec<MenuItem> {
        let (me, app) = (this.clone(), devdeck_core::notes::project_of_node(node));
        vec![MenuItem::new("New note", move |window, cx| Sidebar::new_note(&me, &app, window, cx))]
    }

    fn pins(&self) -> std::collections::BTreeSet<String> {
        self.state.pins.get(self.activity().list_id()).cloned().unwrap_or_default()
    }

    fn running_only(&self) -> bool {
        self.running_only && self.activity() == Activity::Apps
    }

    /// Filtering or "running only": the tree shows the matches, all open.
    fn narrowing(&self, cx: &App) -> bool {
        !self.filter.read(cx).text().trim().is_empty() || self.running_only()
    }

    pub fn rows(&self, cx: &App) -> (Vec<ListItem>, Vec<Row>) {
        let tree = self.project_tree(self.activity(), cx);
        let projects = tree.is_some();
        let (items, names) = match tree {
            Some(t) => (t.items, t.names),
            None => (self.items(cx), Default::default()),
        };
        let id = self.activity().list_id();
        let (none, no_groups) = (std::collections::BTreeSet::new(), Groups::default());
        let shape = Shape {
            open: self.state.open.get(id).unwrap_or(&none),
            groups: self.groups().unwrap_or(&no_groups),
            folded: self.state.collapsed.get(id).unwrap_or(&none),
            names: &names,
            // Projects, then the notes or sessions without one.
            folders_first: projects,
        };
        let rows = list::rows(&items, &self.pins(), self.filter.read(cx).text(), self.running_only(), shape);
        (items, rows)
    }

    /// Lists but Terminals (their numbers come and go, as for pins) and Notes (they stay
    /// with their project) hold groups.
    pub fn groups_allowed(&self) -> bool {
        Self::groups_allowed_in(self.activity())
    }

    /// Whether `list` has groups: terminals come and go, notes go by project.
    pub fn groups_allowed_in(list: Activity) -> bool {
        !matches!(list, Activity::Terminals | Activity::Notes | Activity::Agents)
    }

    /// The shown list's groups.
    pub fn groups(&self) -> Option<&Groups> {
        self.groups_allowed().then(|| self.state.groups.get(self.activity().list_id())).flatten()
    }

    /// A list's groups and open rows, to change.
    fn groups_mut(&mut self, list: Activity) -> (&mut Groups, &mut std::collections::BTreeSet<String>) {
        let id = list.list_id();
        (self.state.groups.entry(id.to_string()).or_default(), self.state.open.entry(id.to_string()).or_default())
    }

    /// The group `node` shows in, itself or with what holds it.
    fn group_of(&self, node: &str) -> Option<String> {
        self.groups()?.shown_in(node).map(String::from)
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.drop_hidden_selection(cx);
        self.save(cx);
        cx.notify();
    }

    /// Folds or unfolds a section. While filtering everything shows: nothing to fold.
    pub fn toggle_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if self.narrowing(cx) {
            return;
        }
        let folded = self.state.collapsed.entry(self.activity().list_id().to_string()).or_default();
        if !folded.remove(section.id()) {
            folded.insert(section.id().to_string());
        }
        self.changed(cx);
    }

    /// Makes a group of `list` in `parent` (or at the top), opened with the way to it;
    /// returns its path. Every group change names its list, shown or not.
    pub fn create_group(&mut self, list: Activity, parent: Option<&str>, name: &str, cx: &mut Context<Self>) -> Result<String, String> {
        if !Self::groups_allowed_in(list) {
            return Err(format!("{} aren't grouped", list.id()));
        }
        let (groups, open) = self.groups_mut(list);
        let path = groups.create(parent, name)?;
        let mut at = Some(path.as_str());
        while let Some(p) = at {
            open.insert(group_row_path(p));
            at = p.rsplit_once('/').map(|(up, _)| up);
        }
        self.changed(cx);
        Ok(path)
    }

    /// Puts a node (an item's key, a folder's path) in a group, or at the top with
    /// `None`; a pinned item is unpinned into it.
    pub fn put(&mut self, list: Activity, node: &str, group: Option<&str>, cx: &mut Context<Self>) {
        if !Self::groups_allowed_in(list) {
            return;
        }
        if let Some(pins) = self.state.pins.get_mut(list.list_id()) {
            pins.remove(node);
        }
        self.groups_mut(list).0.assign(node, group);
        self.changed(cx);
    }

    /// Keeps the selection on a group (or one inside it) of the shown list that moved from
    /// `from` to `to`.
    fn reselect_group(&mut self, list: Activity, from: &str, to: &str) {
        if list != self.activity() {
            return;
        }
        let Some(selected) = self.selected.as_deref().and_then(|k| group_of_row(k.strip_suffix('/')?)) else { return };
        let rest = if selected == from { Some("") } else { selected.strip_prefix(from).filter(|r| r.starts_with('/')) };
        if let Some(rest) = rest {
            self.selected = Some(list::folder_key(&group_row_path(&format!("{to}{rest}"))));
        }
    }

    pub fn rename_group(&mut self, list: Activity, path: &str, name: &str, cx: &mut Context<Self>) -> Result<String, String> {
        let (groups, open) = self.groups_mut(list);
        let to = groups.rename(path, name, open)?;
        self.reselect_group(list, path, &to);
        self.changed(cx);
        Ok(to)
    }

    /// Moves a group into `parent` (or to the top).
    pub fn move_group(&mut self, list: Activity, path: &str, parent: Option<&str>, cx: &mut Context<Self>) -> Result<String, String> {
        let (groups, open) = self.groups_mut(list);
        let to = groups.move_group(path, parent, open)?;
        self.reselect_group(list, path, &to);
        self.changed(cx);
        Ok(to)
    }

    /// Deletes a group; what it holds moves to its parent.
    pub fn delete_group(&mut self, list: Activity, path: &str, cx: &mut Context<Self>) {
        let (groups, open) = self.groups_mut(list);
        for (from, to) in groups.delete(path, open) {
            self.reselect_group(list, &from, &to);
        }
        self.changed(cx);
    }

    /// What dropping an item (by key) on `target` does, if anything.
    pub fn item_drop(&self, key: &str, pinned: bool, target: &Target) -> Option<Effect> {
        if self.activity() == Activity::Terminals {
            return None;
        }
        // Without groups a drop only pins or unpins.
        if !self.groups_allowed() {
            let to_pinned = *target == Target::Pinned;
            return (to_pinned != pinned).then(|| Effect::TogglePin(key.into()));
        }
        match target {
            Target::Pinned => (!pinned).then(|| Effect::TogglePin(key.into())),
            Target::AllHeader if pinned => Some(Effect::Put(key.into(), None)),
            Target::AllHeader => self.group_of(key).is_some().then(|| Effect::Put(key.into(), None)),
            // Unpinned on a row at the top, it goes back to its group.
            Target::All(None) if pinned => Some(Effect::TogglePin(key.into())),
            Target::All(g) if pinned => Some(Effect::Put(key.into(), g.clone())),
            Target::All(g) => (self.group_of(key) != *g).then(|| Effect::Put(key.into(), g.clone())),
        }
    }

    /// What dropping a folder or group row (by its row path) on `target` does.
    pub fn folder_drop(&self, path: &str, target: &Target) -> Option<Effect> {
        let g = match target {
            Target::Pinned => return None,
            Target::All(g) => g,
            Target::AllHeader => &None,
        };
        if !self.groups_allowed() {
            return None;
        }
        match group_of_row(path) {
            Some(p) => {
                let inside = g.as_deref().is_some_and(|g| g == p || g.starts_with(&format!("{p}/")));
                let parent = p.rsplit_once('/').map(|(up, _)| up);
                (!inside && parent != g.as_deref()).then(|| Effect::MoveGroup(p.into(), g.clone()))
            }
            None => (self.group_of(path) != *g).then(|| Effect::Put(path.into(), g.clone())),
        }
    }

    pub fn apply_drop(&mut self, effect: Effect, cx: &mut Context<Self>) {
        match effect {
            Effect::TogglePin(key) => self.toggle_pin(&key, cx),
            Effect::Put(node, g) => self.put(self.activity(), &node, g.as_deref(), cx),
            Effect::MoveGroup(path, g) => {
                if let Err(e) = self.move_group(self.activity(), &path, g.as_deref(), cx) {
                    toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, e, vec![], cx));
                }
            }
        }
    }

    /// Asks for a group's name, makes it in `parent` and moves `then` into it.
    pub fn prompt_new_group(this: &Entity<Self>, parent: Option<String>, then: Option<Moving>, window: &mut Window, cx: &mut App) {
        let me = this.clone();
        let title = match &parent {
            Some(p) => format!("New group in {p}"),
            None => "New group".into(),
        };
        let submit: crate::name_prompt::Submit = std::rc::Rc::new(move |name, cx| {
            me.update(cx, |s, cx| {
                let path = s.create_group(s.activity(), parent.as_deref(), name, cx)?;
                match &then {
                    Some(Moving::Node(n)) => s.put(s.activity(), n, Some(&path), cx),
                    Some(Moving::Group(g)) => {
                        s.move_group(s.activity(), g, Some(&path), cx)?;
                    }
                    None => {}
                }
                Ok(())
            })
        });
        crate::name_prompt::open(title, "", "group-name", submit, window, cx);
    }

    pub fn prompt_rename_group(this: &Entity<Self>, path: String, window: &mut Window, cx: &mut App) {
        let me = this.clone();
        let name = path.rsplit_once('/').map_or(path.as_str(), |(_, l)| l).to_string();
        let title = format!("Rename group {path}");
        let submit: crate::name_prompt::Submit = std::rc::Rc::new(move |name, cx| me.update(cx, |s, cx| s.rename_group(s.activity(), &path, name, cx).map(drop)));
        crate::name_prompt::open(title, &name, "group-name", submit, window, cx);
    }

    /// "Move to group ▸": the item itself makes a new group and moves `what` in; the
    /// submenu has the top level and every group (but `what` and what's inside it).
    pub fn move_menu(this: &Entity<Self>, what: Moving, cx: &App) -> MenuItem {
        let s = this.read(cx);
        let paths = s.groups().map(Groups::paths).unwrap_or_default();
        let (current, skip) = match &what {
            Moving::Node(n) => (s.group_of(n), None),
            Moving::Group(g) => (g.rsplit_once('/').map(|(up, _)| up.to_string()), Some(g.clone())),
        };
        let to = |group: Option<String>| {
            let (me, what) = (this.clone(), what.clone());
            move |_: &mut Window, cx: &mut App| {
                me.update(cx, |s, cx| match &what {
                    Moving::Node(n) => s.put(s.activity(), n, group.as_deref(), cx),
                    Moving::Group(g) => {
                        if let Err(e) = s.move_group(s.activity(), g, group.as_deref(), cx) {
                            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, e, vec![], cx));
                        }
                    }
                })
            }
        };
        let mut sub = vec![MenuItem::new("Top level", to(None)).checked(current.is_none())];
        sub.extend(
            paths
                .into_iter()
                .filter(|p| skip.as_ref().is_none_or(|g| p != g && !p.starts_with(&format!("{g}/"))))
                .map(|p| MenuItem::new(p.clone(), to(Some(p.clone()))).checked(current.as_deref() == Some(p.as_str()))),
        );
        let (me, then) = (this.clone(), what);
        MenuItem::new("Move to group", move |window, cx| Sidebar::prompt_new_group(&me, None, Some(then.clone()), window, cx))
            .submenu(sub)
    }

    /// A group row's menu.
    pub fn group_menu(this: &Entity<Self>, path: &str, cx: &App) -> Vec<MenuItem> {
        let (m1, m2, m3) = (this.clone(), this.clone(), this.clone());
        let (p1, p2, p3) = (path.to_string(), path.to_string(), path.to_string());
        vec![
            MenuItem::new("New group inside", move |window, cx| Sidebar::prompt_new_group(&m1, Some(p1.clone()), None, window, cx)),
            MenuItem::new("Rename", move |window, cx| Sidebar::prompt_rename_group(&m2, p2.clone(), window, cx)),
            MenuItem::new("Delete", move |_, cx| m3.update(cx, |s, cx| s.delete_group(s.activity(), &p3, cx))),
            MenuItem::separator(),
            Sidebar::move_menu(this, Moving::Group(path.to_string()), cx),
        ]
    }

    fn selected_row(&self, items: &[ListItem], rows: &[Row]) -> Option<usize> {
        let key = self.selected.as_deref()?;
        rows.iter().position(|r| row_key(items, r).as_deref() == Some(key))
    }

    /// A selection the list no longer shows goes, except on a full view: its details
    /// stay, whatever a closed folder or the filter hides.
    fn drop_hidden_selection(&mut self, cx: &App) {
        if self.activity().full_view() && self.selected.as_ref().is_some_and(|k| self.view_ref(k, cx).is_some()) {
            return;
        }
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
            Activity::Cluster => ViewAddr::Svc { env: self.list_env(cx).unwrap_or_default(), name: key.into() },
            Activity::Ports => ViewAddr::Port { port: key.into() },
            Activity::Layouts => return None,
            // The tab a layout shows it by, so opening it never adds a second one.
            Activity::Terminals => return self.deck.read(cx).terminals(cx).remove(&key.parse().ok()?).map(|t| t.view),
            Activity::Notes => ViewAddr::Note { id: key.into() },
            Activity::Agents => ViewAddr::Agent { id: key.into() },
        }
        .to_ref())
    }

    /// Stops terminal `n`'s shell; its tabs stay, showing it ended (R6.12).
    pub fn stop_terminal(&mut self, n: u32, cx: &mut Context<Self>) {
        let runner = self.services.read(cx).runner.clone();
        runner.update(cx, |r, cx| r.stop(&RunKey::terminal(n), cx));
    }

    /// Opens an item in the main window's focused pane; an open one is activated. On a
    /// full view it's only selected: its details show.
    pub fn open(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.activity().full_view() {
            self.selected = Some(key.to_string());
            cx.notify();
            return;
        }
        let Some(view) = self.view_ref(key, cx) else {
            if self.activity() != Activity::Layouts {
                return;
            }
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
        let list = self.activity();
        let pinned = self.state.pins.get(list.list_id()).is_some_and(|p| p.contains(key));
        let _ = self.set_pinned(list, key, !pinned, cx);
    }

    /// Pins or unpins an item of `list`, shown or not; nothing changes when it already is.
    pub fn set_pinned(&mut self, list: Activity, key: &str, pinned: bool, cx: &mut Context<Self>) -> Result<(), String> {
        // Terminals come and go with their numbers: nothing to pin.
        if list == Activity::Terminals {
            return Err("terminals aren't pinned".into());
        }
        let pins = self.state.pins.entry(list.list_id().to_string()).or_default();
        if pins.contains(key) != pinned {
            list::toggle_pin(pins, key);
            self.save(cx);
            cx.notify();
        }
        Ok(())
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

    /// Shows `activity` with `key` selected (on a full view, its details).
    pub fn show_item(&mut self, activity: Activity, key: &str, cx: &mut Context<Self>) -> bool {
        if !self.show(activity, cx) {
            return false;
        }
        self.selected = Some(key.to_string());
        let (items, rows) = self.rows(cx);
        let row = self.selected_row(&items, &rows);
        self.select_row(&items, &rows, row, cx);
        true
    }

    /// Agent sessions of the workspace waiting on you (the Agents icon's count).
    pub fn agents_needing(&self, cx: &App) -> usize {
        self.services.read(cx).agents.read(cx).needing(cx)
    }

    /// Agents: the overview of every session (none selected).
    pub fn show_overview(&mut self, cx: &mut Context<Self>) {
        self.show(Activity::Agents, cx);
        self.selected = None;
        cx.notify();
    }

    /// Cluster: the branch matrix the whole width, or the list and its selection again.
    pub fn toggle_matrix(&mut self, cx: &mut Context<Self>) {
        self.matrix = !self.matrix;
        cx.notify();
    }

    /// "Go there" from another view: `activity` shown with `key` selected (a Cluster
    /// deployment on `env`, the list's env then); until that activity has its own full
    /// view, its item also opens in the pulse, as a click on its row does.
    pub fn pivot(&mut self, activity: Activity, key: &str, env: Option<&str>, cx: &mut Context<Self>) -> bool {
        if activity == Activity::Cluster {
            self.matrix = false;
        }
        if env.is_some() && activity == Activity::Cluster {
            let follows = env == self.services.read(cx).env.as_deref();
            self.set_list_env(if follows { None } else { env }, cx);
        }
        if !self.show_item(activity, key, cx) {
            return false;
        }
        self.open(key, cx);
        true
    }

    /// Shows Pulses when a full view hides the pulse's panes: what opens there next is
    /// seen.
    pub fn show_pulses(&mut self, cx: &mut Context<Self>) {
        if self.activity().full_view() {
            self.show(Activity::Layouts, cx);
        }
    }

    /// Shows an activity (and the sidebar); unknown or unavailable ones are ignored.
    /// Returns whether it is now shown.
    pub fn show(&mut self, activity: Activity, cx: &mut Context<Self>) -> bool {
        if !self.activities(cx).contains(&activity) {
            return false;
        }
        if self.activity() != activity {
            if let Some(key) = self.selected.take() {
                self.left.insert(self.activity(), key);
            }
            self.state.activity = activity.id().into();
            self.selected = self.left.remove(&activity);
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
            self.load_list(cx);
            self.save(cx);
            cx.notify();
        }
    }

    /// Makes `env` the workspace's current env: the services, this list and what's saved,
    /// together (the one way, UI and control API). Unknown envs are refused.
    pub fn set_env(&mut self, env: &str, cx: &mut Context<Self>) -> Result<(), String> {
        if !self.envs(cx).iter().any(|e| e.name == env) {
            return Err(format!("no env {env}"));
        }
        if self.state.env.as_deref() != Some(env) {
            self.state.env = Some(env.into());
            self.services.update(cx, |s, cx| s.set_env(env, cx));
            self.load_list(cx);
            self.drop_hidden_selection(cx);
            self.save(cx);
            cx.notify();
        }
        Ok(())
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
        if this.read(cx).activity() == Activity::Terminals {
            let stop = this.clone();
            let n = key.parse::<u32>().unwrap_or_default();
            let Some(view) = this.read(cx).view_ref(key, cx) else { return Vec::new() };
            // Its full view shows it (the same shell; one not started starts there):
            // elsewhere, into a pulse.
            let mut items = Vec::new();
            let runs = this.read(cx).services.read(cx).runner.read(cx).state(&RunKey::terminal(n)).is_some_and(|s| s == devdeck_core::runs::RunState::Running);
            if runs {
                items.push(MenuItem::new("Stop", move |_, cx| stop.update(cx, |s, cx| s.stop_terminal(n, cx))));
            }
            Sidebar::open_in(this, Some(view), &mut items, cx);
            return items;
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
                delete = delete.disabled("switch to another pulse first");
            }
            return vec![
                MenuItem::new("Switch to it", move |_, cx| {
                    d1.update(cx, |d, cx| {
                        let _ = d.switch_layout(&k1, cx);
                    })
                }),
                MenuItem::new("Rename", move |window, cx| crate::layouts::open_rename(d2.clone(), k2.clone(), window, cx)),
                delete,
                MenuItem::separator(),
                Sidebar::move_menu(this, Moving::Node(key.to_string()), cx),
            ];
        }
        let (open, pin) = (this.clone(), this.clone());
        let (k1, k2) = (key.to_string(), key.to_string());
        if this.read(cx).activity() == Activity::Agents {
            // A project's row (the menu key on a selected folder).
            if let Some(node) = key.strip_suffix('/') {
                return Sidebar::agents_project_menu(this, node, cx);
            }
            let agents = this.read(cx).services.read(cx).agents.clone();
            let running = agents.read(cx).running(key, cx);
            let (a1, a2, a3) = (agents.clone(), agents.clone(), agents);
            let (k3, k4, k5) = (key.to_string(), key.to_string(), key.to_string());
            let mut items = vec![
                MenuItem::new(if pinned { "Unpin" } else { "Pin" }, move |_, cx| pin.update(cx, |s, cx| s.toggle_pin(&k2, cx))),
                MenuItem::new("Rename", move |window, cx| rename_session(a1.clone(), k3.clone(), window, cx)),
            ];
            if running {
                items.push(MenuItem::new("Stop", move |_, cx| a2.update(cx, |a, cx| a.stop(&k4, cx))));
            }
            // The agent keeps the conversation: only devdeck's record goes.
            items.push(MenuItem::new("Remove", move |_, cx| a3.update(cx, |a, cx| a.remove(&k5, cx))));
            Sidebar::open_in(this, this.read(cx).view_ref(key, cx), &mut items, cx);
            return items;
        }
        if this.read(cx).activity() == Activity::Notes {
            // A project's row (the menu key on a selected folder).
            if let Some(node) = key.strip_suffix('/') {
                return Sidebar::project_menu(this, node);
            }
            let notes = this.read(cx).services.read(cx).notes.clone();
            let (n1, n2, k3, k4) = (notes.clone(), notes, key.to_string(), key.to_string());
            let mut items = vec![
                MenuItem::new(if pinned { "Unpin" } else { "Pin" }, move |_, cx| pin.update(cx, |s, cx| s.toggle_pin(&k2, cx))),
                MenuItem::new("Rename", move |window, cx| crate::views::note_view::rename_note(&n1, &k3, window, cx)),
                MenuItem::new("Delete", move |window, cx| crate::views::note_view::delete_note(&n2, &k4, window, cx)),
            ];
            // Its full view shows it: elsewhere, into a pulse.
            Sidebar::open_in(this, this.read(cx).view_ref(key, cx), &mut items, cx);
            return items;
        }
        let pin_item = MenuItem::new(if pinned { "Unpin" } else { "Pin" }, move |_, cx| pin.update(cx, |s, cx| s.toggle_pin(&k2, cx)));
        // A full view shows a row's details: nothing to open.
        let full = this.read(cx).activity().full_view();
        // An app's shell and sessions come with it (its own folder; not a folder row's).
        if full {
            let mut items = vec![pin_item, Sidebar::move_menu(this, Moving::Node(key.to_string()), cx)];
            Sidebar::open_in(this, this.read(cx).view_ref(key, cx), &mut items, cx);
            items
        } else {
            vec![
                MenuItem::new("Open", move |_, cx| open.update(cx, |s, cx| s.open(&k1, cx))),
                pin_item,
                Sidebar::move_menu(this, Moving::Node(key.to_string()), cx),
            ]
        }
    }

    pub fn env_menu(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let s = this.read(cx);
        let current = s.state.env.clone();
        let err = theme(cx).err;
        s.envs(cx)
            .iter()
            .map(|e| {
                let (sidebar, name) = (this.clone(), e.name.clone());
                let item = MenuItem::new(e.name.clone(), move |_, cx| {
                    let _ = sidebar.update(cx, |s, cx| s.set_env(&name, cx));
                }).checked(current.as_deref() == Some(&e.name));
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
        if activity == Activity::Terminals && (activity.full_view() || self.state.visible) && self.tick.is_none() {
            self.tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TERMINALS_TICK).await;
                    let shown = this.update(cx, |s, cx| {
                        let on = s.activity() == Activity::Terminals;
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
                // The list's own env (a dropdown, with "sync with workspace"); the
                // workspace's stays in the title bar (user, 2026-11-09).
                let env = self.list_env(cx);
                let protected = env.as_ref().is_some_and(|e| self.envs(cx).iter().any(|x| &x.name == e && x.protected));
                let synced = self.list_synced();
                let workspace = self.services.read(cx).env.clone().unwrap_or_else(|| "-".into());
                let why = if synced { "the env this list shows: the workspace's".to_string() } else { format!("this list's own env; the workspace targets @{workspace}") };
                let me = cx.entity();
                // Its own env: the button highlighted (the tooltip says the workspace's).
                let text = format!("@{}", env.as_deref().unwrap_or("-"));
                let button = if synced { devdeck_ui::button("cluster-env", text, &t) } else { toggle_button("cluster-env", text, true, &t) };
                d.child(
                    button
                        .flex_none()
                        .text_color(if protected { t.err } else { t.env })
                        .debug_selector(|| "cluster-env".into())
                        .tooltip(tip(why))
                        .on_click(move |e, window, cx| open_menu(e.position(), Sidebar::list_env_menu(&me, cx), window, cx))
                        .child(icon(Icon::Open, t.dim, &t)),
                )

                    .child(
                        devdeck_ui::icon_action("open-matrix", Icon::Matrix, "the branch matrix: every service on every env", true, &t)
                            .debug_selector(|| "open-matrix".into())
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_matrix(cx))),                    )
            })
            .when(activity == Activity::Ports, |d| {
                d.child(
                    toggle_button("ports-machine", "machine", self.state.ports_machine, &t)
                        .debug_selector(|| "ports-machine".into())
                        .tooltip(tip("everything listening on this machine, not only this workspace's"))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_ports_machine(cx))),
                )
            })
            .when(activity == Activity::Cluster, |d| {
                d.child(
                    toggle_button("watch-pods", "watch", self.state.watch_pods, &t)
                        .tooltip(tip("watch every pod of this env and mark deployments in trouble"))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_watch_pods(cx))),
                )
            })
            .when(activity == Activity::Notes, |d| {
                d.child(
                    devdeck_ui::button("new-note", "new note", &t)
                        .debug_selector(|| "sidebar-new-note".into())
                        .tooltip(tip("a note without a project"))
                        .on_click(cx.listener(|_, _, window, cx| {
                            let me = cx.entity();
                            window.defer(cx, move |window, cx| Sidebar::new_note(&me, "", window, cx));
                        })),
                )
            })
            .when(activity == Activity::Agents, |d| {
                d.child(
                    devdeck_ui::button("agents-overview", "overview", &t)
                        .debug_selector(|| "sidebar-agents-overview".into())
                        .tooltip(tip("every session at a glance"))
                        .on_click(cx.listener(|this, _, _, cx| this.show_overview(cx))),
                )
                .child(
                    devdeck_ui::button("new-session", "new session", &t)
                        .debug_selector(|| "sidebar-new-session".into())
                        .tooltip(tip("a session of an agent, in one of the workspace's projects"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            let deck = this.deck.clone();
                            window.defer(cx, move |window, cx| pick_new_session(&deck, window, cx));
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

        let list = scroll_list(
            "sidebar-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                let t = theme(cx).clone();
                let (items, rows) = this.rows(cx);
                let status = this.status(cx);
                let selected = this.selected_row(&items, &rows);
                let focused = this.list_focus.is_focused(window);
                let me = cx.entity();
                let quiet_warning = this.activity() == Activity::Apps;
                let notes = this.project_tree(this.activity(), cx);
                let look = Look { filter: filter.clone(), running_only: this.running_only(), focused, status, groups: this.groups_allowed(), quiet_warning, notes, t };
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
        let list = devdeck_ui::with_scrollbar("sidebar-scrollbar", list, &devdeck_ui::list_handle(&self.scroll)).flex_1();

        div()
            .id("sidebar")
            .key_context("Sidebar")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(t.bar)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.select_edge(false, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.select_edge(true, cx)))
            .on_action(cx.listener(|this, _: &OpenSelected, _, cx| this.open_selected(cx)))
            .on_action(cx.listener(|this, _: &ExpandSelected, _, cx| this.expand_selected(cx)))
            .on_action(cx.listener(|this, _: &CollapseSelected, _, cx| this.collapse_selected(cx)))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| this.toggle_pin_selected(cx)))
            .on_action(cx.listener(|this, _: &ToggleRunningOnly, _, cx| this.toggle_running_only(cx)))
            .on_action(cx.listener(|this, _: &ToggleWatchPods, _, cx| this.toggle_watch_pods(cx)))
            .on_action(cx.listener(|this, _: &OpenListEnvMenu, window, cx| this.open_list_env_menu(window, cx)))
            .on_action(cx.listener(|this, _: &NewNote, window, cx| {
                if this.activity() == Activity::Notes {
                    let me = cx.entity();
                    window.defer(cx, move |window, cx| Sidebar::new_note(&me, "", window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                this.filter.read(cx).focus_handle(cx).focus(window, cx);
            }))
            .on_action(cx.listener(|this, _: &SidebarEscape, window, cx| this.escape(window, cx)))
            .child(header)
            .when_some(self.kept_problem(cx), |d, p| {
                d.child(div().flex_none().px_3().pb_1().debug_selector(|| "sidebar-problem".into()).child(icon_text(Icon::Warning, t.warn, p, &t)))
            })
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
    /// The list holds groups (a menu on the All header).
    groups: bool,
    /// A warning is ⚠ alone at the row's right, its reason in the tooltip (the Apps
    /// list); else ⚠ in the state slot and its reason as the right text.
    quiet_warning: bool,
    /// The Notes list's tree: its folders are projects (named, hinted, dim once gone).
    notes: Option<devdeck_core::notes::Tree>,
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
    // Every row takes drops for where it is (one handler per dragged type: GPUI panics
    // on two): pin, unpin, into its group or to the top.
    let row_drop = |el: gpui::Stateful<gpui::Div>| {
        let target = target(rows, i);
        let bg = t.drop_bg;
        let (m1, m2, m3, m4) = (me.clone(), me.clone(), me.clone(), me.clone());
        let (t1, t2, t3, t4) = (target.clone(), target.clone(), target.clone(), target);
        el.drag_over::<DraggedListItem>(move |s, d: &DraggedListItem, _, cx| {
            if d.sidebar == sidebar_id && m1.read(cx).item_drop(&d.key, d.pinned, &t1).is_some() { s.bg(bg) } else { s }
        })
        .drag_over::<DraggedFolder>(move |s, d: &DraggedFolder, _, cx| {
            if d.sidebar == sidebar_id && m2.read(cx).folder_drop(&d.path, &t2).is_some() { s.bg(bg) } else { s }
        })
        .on_drop(move |d: &DraggedListItem, _, cx| {
            if d.sidebar == sidebar_id {
                m3.update(cx, |s, cx| {
                    if let Some(e) = s.item_drop(&d.key, d.pinned, &t3) {
                        s.apply_drop(e, cx);
                    }
                });
            }
        })
        .on_drop(move |d: &DraggedFolder, _, cx| {
            if d.sidebar == sidebar_id {
                m4.update(cx, |s, cx| {
                    if let Some(e) = s.folder_drop(&d.path, &t4) {
                        s.apply_drop(e, cx);
                    }
                });
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
        Row::Header { section, open, count } => {
            let text = match section {
                Section::Pinned => format!("PINNED · {count}"),
                Section::All => format!("ALL · {count}"),
            };
            let selector = if section == Section::Pinned { "header-pinned" } else { "header-all" };
            let (m_chevron, m_click, m_menu) = (me.clone(), me.clone(), me.clone());
            let el = tree_row(selector, 0, Some(open), move |_, cx| m_chevron.update(cx, |s, cx| s.toggle_section(section, cx)), t)
                .cursor_pointer()
                .text_color(t.dim)
                .text_size(t.text_small)
                .on_mouse_down(MouseButton::Left, move |_, _, cx| m_click.update(cx, |s, cx| s.toggle_section(section, cx)))
                .when(section == Section::All && (look.groups || look.notes.is_some()), |d| {
                    let notes = look.notes.is_some();
                    d.on_mouse_down(MouseButton::Right, move |e, window, cx| {
                        let m = m_menu.clone();
                        let items = if notes {
                            vec![MenuItem::new("New note", move |window, cx| Sidebar::new_note(&m, "", window, cx))]
                        } else {
                            vec![MenuItem::new("New group", move |window, cx| Sidebar::prompt_new_group(&m, None, None, window, cx))]
                        };
                        open_menu(e.position, items, window, cx);
                    })
                })
                .child(text);
            row_drop(el).into_any_element()
        }
        Row::Empty(section) => {
            row_drop(base.id(("empty", i)).pl_6().text_color(t.dim).child(label(empty_text(section, filtered, look.running_only, look.status.as_deref())))).into_any_element()
        }
        Row::Folder { path, name, depth, kids, group } => {
            let key = list::folder_key(&path);
            let (m, m_menu) = (me.clone(), me.clone());
            let k_menu = key.clone();
            let menu_path = path.clone();
            let drag = DraggedFolder { sidebar: sidebar_id, path: path.clone(), label: name.clone().into() };
            let project = look.notes.as_ref();
            let hint: SharedString = match project.and_then(|n| n.hints.get(&path)) {
                Some(h) => h.clone().into(),
                None => group_of_row(&path).map_or_else(|| path.clone(), |g| format!("group {g}")).into(),
            };
            let gone = project.is_some_and(|n| n.gone.contains(&path));
            let el = tree(&key, depth, Some(kids), Some(path.clone()))
                .tooltip(tip(hint))
                .on_drag(drag, |d, _, _, cx| cx.new(|_| DragGhost(d.label.clone())))
                .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                    m_menu.update(cx, |s, cx| {
                        s.selected = Some(k_menu.clone());
                        cx.notify();
                    });
                    let list = m_menu.read(cx).activity();
                    let items = match group_of_row(&menu_path) {
                        None if list == Activity::Notes => Sidebar::project_menu(&m_menu, &menu_path),
                        None if list == Activity::Agents => Sidebar::agents_project_menu(&m_menu, &menu_path, cx),
                        Some(g) => Sidebar::group_menu(&m_menu, g, cx),
                        None if m_menu.read(cx).groups_allowed() => vec![Sidebar::move_menu(&m_menu, Moving::Node(menu_path.clone()), cx)],
                        None => Vec::new(),
                    };
                    if !items.is_empty() {
                        open_menu(e.position, items, window, cx);
                    }
                })
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
                .when(group, |d| d.child(icon(Icon::Group, t.mute, t)))
                .child(div().flex_1().min_w_0().truncate().text_color(if gone { t.dim } else { t.mute }).child(if group || project.is_some() { name } else { format!("{name}/") }))
                .when(!kids.open, |d| d.child(holds(kids, t)));
            row_drop(el).into_any_element()
        }
        Row::Item { index, pinned, depth, kids } => {
            let item = &items[index];
            let key = item.key.clone();
            let (k_click, k_menu) = (key.clone(), key.clone());
            let (m_click, m_menu) = (me.clone(), me.clone());
            let label_text: SharedString = item.label.clone().into();
            let drag = view_ref.map(|view| DraggedListItem { sidebar: sidebar_id, deck: deck.entity_id(), key: key.clone(), label: label_text.clone(), pinned, view });
            // Shown only to hold matches below it, or a library: de-emphasized.
            let context_only = !list::keeps(item, &look.filter, look.running_only);
            let text_color = if context_only { t.dim } else if item.warning.is_some() || item.dim { t.mute } else { t.fg };
            let el = tree(&key, depth, kids, rows[i].open_path(items).map(String::from))
                .tooltip(tip(item.hint.clone().map_or_else(|| label_text.clone(), SharedString::from)))
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
                    Some(_) if !look.quiet_warning => icon(Icon::Warning, t.warn, t).into_any_element(),
                    _ => dot(if item.running { t.ok } else { t.dim }, item.running, t).into_any_element(),
                })
                .child(div().flex_1().min_w_0().truncate().text_color(text_color).child(label_text))
                .when_some(kids.filter(|k| !k.open), |d, k| d.child(holds(k, t)))
                .when(item.busy, |d| d.child(icon(Icon::Busy, t.accent, t)))
                .when_some(item.pods, |d, (ready, wanted, failing)| {
                    let other = if failing { devdeck_ui::PodMark::Failing } else { devdeck_ui::PodMark::Starting };
                    let marks: Vec<devdeck_ui::PodMark> = (0..wanted.min(12)).map(|i| if i < ready { devdeck_ui::PodMark::Ready } else { other }).collect();
                    d.child(devdeck_ui::pods_bar(&format!("row-pods-{key}"), &marks, t))
                })
                .when_some(item.warning.clone().filter(|_| !look.quiet_warning).map(|w| (w, t.warn)).or((!item.detail.is_empty()).then(|| (item.detail.clone(), t.dim))), |d, (detail, color)| {
                    d.child(div().flex_none().max_w(gpui::relative(0.6)).truncate().text_color(color).child(detail))
                })
                .when(look.quiet_warning && item.warning.is_some(), |d| {
                    d.child(icon(Icon::Warning, t.warn, t).debug_selector(|| "row-warning".into()))
                });
            row_drop(el).into_any_element()
        }
    }
}

/// Where a drop on row `i` lands: Pinned, or All in the group the row is (or is in).
fn target(rows: &[Row], i: usize) -> Target {
    let section = rows[..=i].iter().rev().find_map(|r| match r {
        Row::Header { section, .. } => Some(*section),
        _ => None,
    });
    if section != Some(Section::All) {
        return Target::Pinned;
    }
    let group = |path: &str| Target::All(group_of_row(path).map(String::from));
    let mut depth = match &rows[i] {
        Row::Folder { path, group: true, .. } => return group(path),
        Row::Header { .. } => return Target::AllHeader,
        Row::Empty(_) => return Target::All(None),
        r => r.depth(),
    };
    for r in rows[..i].iter().rev() {
        match r {
            Row::Header { .. } => break,
            Row::Folder { path, group: true, depth: d, .. } if *d < depth => return group(path),
            r if r.depth() < depth => depth = r.depth(),
            _ => {}
        }
    }
    Target::All(None)
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

/// A new session of agent `agent` in `dir` (R6.14), shown in its workspace's Agents view.
pub fn new_agent_session(deck: &Entity<Deck>, agent: &str, dir: &std::path::Path, cx: &mut App) {
    let Some(services) = deck.read(cx).services() else { return };
    let n = deck.read(cx).next_terminal(cx);
    let agents = services.read(cx).agents.clone();
    let id = agents.update(cx, |a, cx| a.create(agent, dir, n, cx));
    let sidebar = crate::shell::try_shell(cx).and_then(|s| s.read(cx).workspaces.iter().find(|w| w.deck == *deck).map(|w| w.sidebar.clone()));
    // Once the caller is done: it may be the sidebar itself (its new session button).
    if let Some(sidebar) = sidebar {
        cx.defer(move |cx| {
            sidebar.update(cx, |s, cx| s.show_item(Activity::Agents, &id, cx));
        });
    }
}

/// Asks a session's name (empty: its agent's title again).
fn rename_session(agents: Entity<crate::agents::Agents>, id: String, window: &mut Window, cx: &mut App) {
    let name = agents.read(cx).session(&id).and_then(|s| s.name.clone()).unwrap_or_else(|| agents.read(cx).label(&id, cx));
    let submit: crate::name_prompt::Submit = std::rc::Rc::new(move |to, cx| {
        agents.update(cx, |a, cx| a.rename(&id, to, cx));
        Ok(())
    });
    crate::name_prompt::open("Rename session", &name, "session-name", submit, window, cx);
}

/// Picks a project of the workspace, then the agent when there are several, and makes a
/// session there (R6.14).
pub fn pick_new_session(deck: &Entity<Deck>, window: &mut Window, cx: &mut App) {
    use devdeck_ui::picker::{PickGroup, PickOption, PickerSpec, open_picker};
    let kinds = crate::environment::agent_kinds(cx);
    if kinds.is_empty() {
        toasts(cx).update(cx, |t, cx| t.push(ToastKind::Info, "No agent is configured: add one to machine.toml ([agents.<id>]).", vec![], cx));
        return;
    }
    let Some(services) = deck.read(cx).services() else { return };
    let mut apps: Vec<(String, String, std::path::PathBuf)> = services.read(cx).catalog.apps.iter().map(|a| (a.name.clone(), a.folder.clone(), a.dir.clone())).collect();
    apps.sort_by_key(|(name, ..)| name.to_lowercase());
    let options = apps.into_iter().map(|(name, folder, dir)| {
        let mut o = PickOption::new(dir.display().to_string()).detail(folder);
        o.label = name.into();
        o
    });
    let deck = deck.clone();
    let on_pick: devdeck_ui::picker::OnPick = std::rc::Rc::new(move |dir, window, cx| {
        let dir = std::path::PathBuf::from(dir);
        match kinds.as_slice() {
            [one] => new_agent_session(&deck, &one.id, &dir, cx),
            many => {
                let deck = deck.clone();
                let options = many.iter().map(|k| {
                    let mut o = PickOption::new(k.id.clone()).detail(k.command.clone());
                    o.label = k.name.clone().into();
                    o
                });
                let on_agent: devdeck_ui::picker::OnPick = std::rc::Rc::new(move |agent, _, cx| new_agent_session(&deck, &agent, &dir, cx));
                let spec = PickerSpec { title: "New session · agent".into(), placeholder: "agent".into(), kinds: Vec::new(), groups: vec![PickGroup { label: "Agents".into(), options: options.collect() }], typed: None, on_pick: on_agent };
                window.defer(cx, move |window, cx| {
                    open_picker(spec, window, cx);
                });
            }
        }
    });
    let spec = PickerSpec { title: "New session · project".into(), placeholder: "project".into(), kinds: Vec::new(), groups: vec![PickGroup { label: "Projects".into(), options: options.collect() }], typed: None, on_pick };
    open_picker(spec, window, cx);
}
