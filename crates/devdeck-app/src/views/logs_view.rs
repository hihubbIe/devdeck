//! Service logs (`logs/<env>/<name>`, R7.5): every pod's lines with their time and pod,
//! level filter, a pod and a container picked, pause, follow, find with n / shift+n, a
//! crashed pod's previous run. What streams and what's kept: [`super::logs_feed`]; the
//! streams run only while the view is on screen, and it holds its service's pods while
//! open (their containers' states decide what streams).
//! The toolbar keeps find (the free width), the level, wrap, pause, follow and a cog menu
//! for the rest (pod, container, previous run, columns); a warning icon says what's wrong.
//! Keys (`LogsView`): / find, n next, shift+n previous, p pause, 1–4 level filter, w wrap,
//! c container, o pod, shift+p previous run.

use super::ViewCx;
use super::logs_feed::{LogsFeed, match_ranges, short};
use crate::cluster::{Cluster, PodScope, PodsInterest};
use devdeck_core::cluster::{Pod, Remote};
use devdeck_core::runs::Level;
use devdeck_ui::{Count, Fold, Icon, InputEvent, MenuItem, MenuValue, TextInput, ToolAction, ToolItem, Toolbar, ToolbarState, label, open_menu, theme, toasts};
use devdeck_ui::{Scrolls, ToastKind, scroll_list};
use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ListHorizontalSizingBehavior,
    ParentElement, Pixels, Render, ScrollStrategy, SharedString, Styled, Subscription, UniformListScrollHandle,
    Window, actions, canvas, div, prelude::*,
};
use std::cell::Cell;
use std::rc::Rc;

pub use crate::fake_cluster::LOGS_TICK;

actions!(logs_view, [Find, NextMatch, PrevMatch, TogglePause, OnlyErrors, WarnAndUp, InfoAndUp, Everything, PickContainer, PickPod, PreviousRun, ToggleWrap, Options, LevelMenu, ToggleTime, TogglePodColumn]);

/// Lowest level shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MinLevel {
    Error,
    Warn,
    Info,
    All,
}

pub fn shown(level: Level, min: MinLevel) -> bool {
    match min {
        MinLevel::All => true,
        MinLevel::Info => level != Level::Debug,
        MinLevel::Warn => matches!(level, Level::Warn | Level::Error),
        MinLevel::Error => level == Level::Error,
    }
}

/// Wrapped rows: the width in characters and the buffer version they were made for, and
/// each piece's shown row and first character.
type Wrapped = (usize, u64, Vec<(usize, usize)>);

/// What the service's pods looked like last: a change syncs the streams and renders.
type Seen = (Option<Vec<Pod>>, (u64, u64), bool);

pub struct ServiceLogsView {
    pub env: String,
    pub name: String,
    cluster: Entity<Cluster>,
    focus: FocusHandle,
    pub find: Entity<TextInput>,
    pub feed: LogsFeed,
    pub min_level: MinLevel,
    pub follow: bool,
    /// Long lines wrap at the pane's width (R4.4 lets a log view opt in); on by default.
    pub wrap: bool,
    pub show_time: bool,
    pub show_pod: bool,
    /// Wrapped rows: each a shown row and the character its piece starts at, for a width
    /// in characters and a buffer version.
    wrapped: Option<Wrapped>,
    /// The current match: its line (absolute index) and which match of that line.
    pub current: Option<(u64, usize)>,
    scroll: UniformListScrollHandle,
    /// Where the view is in its window.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// The toolbar's last form and its items' places (menus opened by key go under them).
    bar: Rc<ToolbarState>,
    seen: Seen,
    _pods: PodsInterest,
    _subs: Vec<Subscription>,
}

impl ServiceLogsView {
    pub fn new(env: String, name: String, ctx: ViewCx, cluster: Entity<Cluster>, cx: &mut Context<Self>) -> Self {
        let find = cx.new(|cx| TextInput::new("find", cx));
        let mut subs = vec![
            cx.observe(&cluster, |this, _, cx| this.cluster_changed(cx)),
            cx.subscribe(&find, |this, _, e: &InputEvent, cx| match e {
                InputEvent::Changed(text) => {
                    this.feed.buffer.set_query(text);
                    this.current = None;
                    this.step_match(1, cx);
                }
                InputEvent::Submit(_) => this.step_match(1, cx),
                InputEvent::Cancel => {
                    this.find.update(cx, |f, cx| f.set_text("", cx));
                    this.feed.buffer.set_query("");
                    this.current = None;
                    cx.notify();
                }
            }),
        ];
        subs.extend(ctx.on_visibility(cx, |this, visible, cx| {
            let cluster = this.cluster.clone();
            this.feed.set_visible(visible, &cluster, cx);
        }));
        let pods = cluster.update(cx, |c, cx| c.pods_interest(&env, PodScope::Service(name.clone()), cx));
        Self {
            feed: LogsFeed::new(env.clone(), name.clone()),
            env,
            name,
            cluster,
            focus: cx.focus_handle(),
            find,
            min_level: MinLevel::All,
            follow: true,
            wrap: true,
            show_time: true,
            show_pod: true,
            wrapped: None,
            current: None,
            scroll: UniformListScrollHandle::new(),
            bounds: Rc::default(),
            bar: ToolbarState::new(),
            seen: (None, (0, 0), false),
            _pods: pods,
            _subs: subs,
        }
    }

    /// Whether any pod's logs stream now.
    pub fn is_streaming(&self) -> bool {
        self.feed.streams() > 0
    }

    /// Only a change of the service's pods, the env's log generation or its pods' problem
    /// matters here: other cluster changes render nothing.
    fn cluster_changed(&mut self, cx: &mut Context<Self>) {
        let c = self.cluster.read(cx);
        let pods = match c.pods_state(&self.env, &self.name) {
            Remote::Ready(p) => Some(p.to_vec()),
            _ => None,
        };
        let seen = (pods, c.log_generation(&self.env), c.pods_problem(&self.env).is_some());
        if seen == self.seen {
            return;
        }
        self.seen = seen;
        let cluster = self.cluster.clone();
        self.feed.sync(&cluster, cx);
        cx.notify();
    }

    /// Brings the streams in line with the cluster again (a retry timer's).
    pub(crate) fn resync(&mut self, cx: &mut Context<Self>) {
        let cluster = self.cluster.clone();
        self.feed.sync(&cluster, cx);
        cx.notify();
    }

    /// New lines are in: follow them, render once.
    pub(crate) fn after_lines(&mut self, cx: &mut Context<Self>) {
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    /// Matches of the find text among the lines shown.
    pub fn matches(&self) -> usize {
        self.feed.buffer.matches()
    }

    /// The next (or previous) match, every match of every line in turn.
    pub fn step_match(&mut self, dir: isize, cx: &mut Context<Self>) {
        let b = &self.feed.buffer;
        let total = b.matches();
        if total == 0 {
            self.current = None;
            cx.notify();
            return;
        }
        let next = match self.current.and_then(|(line, nth)| b.match_index(line, nth)) {
            None => 0,
            Some(i) => (i as isize + dir).rem_euclid(total as isize) as usize,
        };
        let Some((line, nth)) = b.match_at(next) else { return };
        self.current = Some((line, nth));
        self.follow = false;
        if let Some(row) = b.row_of(line) {
            let item = match &self.wrapped {
                Some((cols, _, rows)) if self.wrap => {
                    // The wrapped piece holding the match's first character.
                    let text = b.row(row).map(|e| e.text.clone()).unwrap_or_default();
                    let start = match_ranges(&text, b.query()).get(nth).map_or(0, |r| text[..r.start].chars().count());
                    rows.partition_point(|(r, _)| *r < row) + start / cols.max(&1)
                }
                _ => row,
            };
            self.scroll.scroll_to_item(item, ScrollStrategy::Center);
        }
        cx.notify();
    }

    pub fn set_min_level(&mut self, l: MinLevel, cx: &mut Context<Self>) {
        self.min_level = l;
        self.feed.buffer.set_min(l);
        self.current = None;
        cx.notify();
    }

    /// Shows one pod's lines only (`None`: every pod's).
    pub fn set_pod(&mut self, pod: Option<SharedString>, cx: &mut Context<Self>) {
        self.feed.buffer.set_pod(pod);
        self.current = None;
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    pub fn set_container(&mut self, container: Option<String>, cx: &mut Context<Self>) {
        let cluster = self.cluster.clone();
        self.current = None;
        self.feed.set_container(container, &cluster, cx);
    }

    pub fn toggle_follow(&mut self, cx: &mut Context<Self>) {
        self.follow = !self.follow;
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    /// Rows on screen's list: wrapped pieces when wrapping (as last drawn), else lines.
    pub fn list_rows(&self) -> usize {
        match &self.wrapped {
            Some((_, _, rows)) if self.wrap => rows.len(),
            _ => self.feed.buffer.row_count(),
        }
    }

    pub fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        self.wrap = !self.wrap;
        self.wrapped = None;
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    pub fn toggle_pause(&mut self, cx: &mut Context<Self>) {
        let paused = !self.feed.paused;
        self.feed.set_paused(paused);
        self.after_lines(cx);
    }

    /// The pod whose previous run "previous run" reads: the one picked, else the only pod
    /// whose container shown restarted; why not, when there's none.
    pub fn previous_target(&self, cx: &App) -> Result<SharedString, String> {
        let pods = self.cluster.read(cx).pods(&self.env, &self.name);
        let restarted = |p: &Pod| self.feed.container_of(p).is_some_and(|c| p.containers.iter().any(|s| s.name == c && s.restarts > 0));
        if let Some(pod) = self.feed.buffer.pod() {
            return match pods.iter().find(|p| p.name == pod.as_ref()) {
                Some(p) if restarted(p) => Ok(pod.clone()),
                Some(_) => Err(format!("pod {} hasn't restarted", short(pod))),
                None => Err(format!("pod {} is gone", short(pod))),
            };
        }
        let mut crashed = pods.iter().filter(|p| restarted(p));
        match (crashed.next(), crashed.next()) {
            (Some(p), None) => Ok(p.name.clone().into()),
            (Some(_), Some(_)) => Err("several pods restarted: pick one first (o)".into()),
            _ => Err("no pod has restarted".into()),
        }
    }

    pub fn previous_run(&mut self, cx: &mut Context<Self>) {
        let result = self.previous_target(cx).and_then(|pod| {
            let cluster = self.cluster.clone();
            self.feed.read_previous(&pod, &cluster, cx)
        });
        if let Err(why) = result {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Info, format!("Previous run: {why}."), vec![], cx));
        }
    }

    /// The containers that can be picked: each pod's main one, then every container the
    /// service's pods run, init containers too (a failing migration).
    pub fn containers(&self, cx: &App) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = Vec::new();
        for p in self.cluster.read(cx).pods(&self.env, &self.name) {
            for c in &p.containers {
                if !out.iter().any(|(n, _)| n == &c.name) {
                    out.push((c.name.clone(), c.init));
                }
            }
        }
        out.sort();
        out
    }

    fn container_menu(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let v = this.read(cx);
        let current = v.feed.container.clone();
        let pick = |c: Option<String>| {
            let me = this.clone();
            move |_: &mut Window, cx: &mut App| me.update(cx, |v, cx| v.set_container(c.clone(), cx))
        };
        let mut items = vec![MenuItem::new("each pod's main container", pick(None)).checked(current.is_none())];
        for (name, init) in v.containers(cx) {
            let text = if init { format!("{name} (init)") } else { name.clone() };
            items.push(MenuItem::new(text, pick(Some(name.clone()))).checked(current.as_deref() == Some(&name)));
        }
        items
    }

    fn pod_menu(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let v = this.read(cx);
        let current = v.feed.buffer.pod().cloned();
        let pick = |p: Option<SharedString>| {
            let me = this.clone();
            move |_: &mut Window, cx: &mut App| me.update(cx, |v, cx| v.set_pod(p.clone(), cx))
        };
        let mut items = vec![MenuItem::new("every pod", pick(None)).checked(current.is_none())];
        for (name, gone) in v.feed.pods() {
            let text = if gone { format!("{} (gone)", short(&name)) } else { short(&name).to_string() };
            items.push(MenuItem::new(text, pick(Some(name.clone()))).checked(current.as_ref() == Some(&name)));
        }
        items
    }

    fn level_menu(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let current = this.read(cx).min_level;
        [(MinLevel::Error, "errors"), (MinLevel::Warn, "warn+"), (MinLevel::Info, "info+"), (MinLevel::All, "all")]
            .into_iter()
            .map(|(l, text)| {
                let me = this.clone();
                MenuItem::new(text, move |_, cx| me.update(cx, |v, cx| v.set_min_level(l, cx))).checked(current == l)
            })
            .collect()
    }

    /// The cog's menu: the pod and container shown, a previous run, the columns, wrapping.
    fn options_menu(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let v = this.read(cx);
        let toggle = |f: fn(&mut Self)| {
            let me = this.clone();
            move |_: &mut Window, cx: &mut App| {
                me.update(cx, |v, cx| {
                    f(v);
                    v.wrapped = None;
                    cx.notify();
                })
            }
        };
        let mut items = vec![MenuItem::new("pod", |_, _| {}).submenu(Self::pod_menu(this, cx))];
        if v.containers(cx).len() > 1 {
            items.push(MenuItem::new("container", |_, _| {}).submenu(Self::container_menu(this, cx)));
        }
        let me = this.clone();
        let previous = MenuItem::new("previous run of a pod", move |_, cx| me.update(cx, |v, cx| v.previous_run(cx)));
        items.push(match v.previous_target(cx) {
            Ok(_) => previous,
            Err(why) => previous.disabled(why),
        });
        items.push(MenuItem::separator());
        items.push(MenuItem::new("show the time", toggle(|v| v.show_time = !v.show_time)).checked(v.show_time));
        items.push(MenuItem::new("show the pod", toggle(|v| v.show_pod = !v.show_pod)).checked(v.show_pod));
        items
    }

    /// Wrapped rows for `cols` characters of text: worked out again only when the width or
    /// what's shown changed.
    fn wrap_rows(&mut self, cols: usize) -> usize {
        let version = self.feed.buffer.version();
        let fresh = self.wrapped.as_ref().is_some_and(|(c, v, _)| *c == cols && *v == version);
        if !fresh {
            let b = &self.feed.buffer;
            let mut rows = Vec::with_capacity(b.row_count());
            for r in 0..b.row_count() {
                let chars = b.row(r).map_or(0, |e| e.text.chars().count());
                let pieces = chars.div_ceil(cols).max(1);
                rows.extend((0..pieces).map(|p| (r, p * cols)));
            }
            self.wrapped = Some((cols, version, rows));
        }
        self.wrapped.as_ref().map_or(0, |(_, _, rows)| rows.len())
    }

    /// The toolbar (R9.9): find, what's wrong, the level, wrap, pause, follow, and "more"
    /// with the pod, the container, a previous run and the columns.
    fn toolbar(&self, cx: &mut Context<Self>) -> Toolbar {
        let t = theme(cx).clone();
        let me = cx.entity();
        // A toolbar shows short words; the palette keeps the registry's full names.
        let label = |id: &str, fallback: &'static str| crate::registry::by_id(id).map_or(fallback, |s| s.label);
        let run = |f: fn(&mut Self, &mut Context<Self>)| {
            let me = me.clone();
            move |_: &mut Window, cx: &mut App| me.update(cx, f)
        };
        let matches = self.matches();
        let count = (matches > 0).then(|| {
            let text = match self.current.and_then(|(line, nth)| self.feed.buffer.match_index(line, nth)) {
                Some(p) => format!("{}/{matches}", p + 1),
                None => format!("{matches}"),
            };
            Count::new(text, 9, t.mute)
        });
        let level = match self.min_level {
            MinLevel::Error => "errors",
            MinLevel::Warn => "warn+",
            MinLevel::Info => "info+",
            MinLevel::All => "all",
        };
        let held = self.feed.held();
        let paused = self.feed.paused;
        let mut bar = Toolbar::new("logs-toolbar", self.bar.clone()).item(ToolItem::Field {
            id: "logs.find".into(),
            label: label("logs.find", "find").into(),
            input: self.find.clone().into(),
            focus: self.find.read(cx).focus_handle(cx),
            min_chars: 12,
            count,
            accessories: Vec::new(),
        });
        // What's wrong, in one status: pods waiting, streams' problems.
        let mut wrong: Vec<String> = self.feed.waiting().iter().map(|(why, n)| format!("{n} waiting: {why}")).collect();
        wrong.extend(self.feed.problems().iter().map(|(pod, why)| format!("{}: {why}", short(pod))));
        if !wrong.is_empty() {
            let word = if wrong.len() == 1 { "1 problem".to_string() } else { format!("{} problems", wrong.len()) };
            let lines = wrong.clone();
            bar = bar.item(ToolItem::Status {
                id: "logs.wrong".into(),
                icon: Icon::Warning,
                color: t.warn,
                word: Some(word.into()),
                tip: wrong.join("\n").into(),
                details: Rc::new(move |_| lines.iter().map(|l| MenuItem::new(l.clone(), |_, _| {}).disabled("")).collect()),
            });
        }
        let me_level = me.clone();
        bar = bar
            .item(ToolItem::Menu {
                action: ToolAction::new("logs.level", "level", run(|v, cx| v.cycle_level(cx))).icon(Icon::Filter).fold(Fold::Order(3)).state_shown(self.min_level != MinLevel::All),
                value: Some(MenuValue::new(level)),
                max_chars: 6,
                items: Rc::new(move |cx| Self::level_menu(&me_level, cx)),
            })
            .item(ToolItem::Action(ToolAction::new("logs.wrap", "wrap", run(|v, cx| v.toggle_wrap(cx))).icon(Icon::Wrap).toggle(self.wrap).fold(Fold::Order(2))))
            .item(ToolItem::Action(
                ToolAction::new("logs.pause", if paused { "resume" } else { "pause" }, run(|v, cx| v.toggle_pause(cx)))
                    .icon(if paused { Icon::Run } else { Icon::Pause })
                    .toggle(paused)
                    .state_shown(paused)
                    .count((paused && held > 0).then(|| Count::new(format!("{} new", if held > 999 { "999+".to_string() } else { held.to_string() }), 8, t.warn)))
                    .fold(Fold::Order(1)),
            ))
            .item(ToolItem::Action(
                ToolAction::new("pane.follow", "follow", run(|v, cx| v.toggle_follow(cx))).icon(Icon::Follow).toggle(self.follow).state_shown(!self.follow).fold(Fold::Order(4)),
            ));
        let c = self.cluster.read(cx);
        let pods = c.pods(&self.env, &self.name).len();
        let mut figures = format!("{pods} pods · {} lines", self.feed.buffer.row_count());
        let dropped = self.feed.buffer.dropped();
        if dropped > 0 {
            figures.push_str(&format!(" · {dropped} older dropped"));
        }
        if let Some(pod) = self.feed.buffer.pod() {
            figures.push_str(&format!(" · pod {}", short(pod)));
        }
        if let Some(container) = &self.feed.container {
            figures.push_str(&format!(" · container {container}"));
        }
        let me_more = me.clone();
        bar.more(Rc::new(move |cx| Self::options_menu(&me_more, cx)), Some(figures.into()))
    }

    /// The level after this one (the level button, run when it can't open its menu).
    fn cycle_level(&mut self, cx: &mut Context<Self>) {
        let next = match self.min_level {
            MinLevel::All => MinLevel::Info,
            MinLevel::Info => MinLevel::Warn,
            MinLevel::Warn => MinLevel::Error,
            MinLevel::Error => MinLevel::All,
        };
        self.set_min_level(next, cx);
    }

    /// Opens a picker's menu under the toolbar (by key) or at the click.
    fn open_picker(&mut self, which: fn(&Entity<Self>, &App) -> Vec<MenuItem>, at: Option<gpui::Point<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        let b = self.bounds.get();
        // By key: under the toolbar's "more", where these live.
        let at = at.or_else(|| self.bar.bounds(devdeck_ui::toolbar::MORE).map(|m| m.bottom_left())).unwrap_or(b.origin);
        let me = cx.entity();
        window.defer(cx, move |window, cx| {
            let items = which(&me, cx);
            open_menu(at, items, window, cx)
        });
    }
}

/// Opens (or shows) a service's logs showing only `pod`'s lines (the pod row's "Logs").
pub fn open_for_pod(ctx: &ViewCx, logs: devdeck_core::view_addr::ViewAddr, pod: &str, cx: &mut App) {
    ctx.open(logs.clone(), cx);
    let view = ctx.deck.upgrade().and_then(|d| d.read(cx).view(&logs.to_ref())).and_then(|v| v.downcast::<ServiceLogsView>().ok());
    if let Some(v) = view {
        let pod = SharedString::from(pod.to_string());
        v.update(cx, |v, cx| v.set_pod(Some(pod), cx));
    }
}

/// The level shown, the find text, follow, wrap and the columns; never paused (lines would
/// pile up unseen).
impl super::KeepsState for ServiceLogsView {
    fn save(&self, cx: &App) -> serde_json::Value {
        let level = match self.min_level {
            MinLevel::Error => "error",
            MinLevel::Warn => "warn",
            MinLevel::Info => "info",
            MinLevel::All => "all",
        };
        serde_json::json!({ "level": level, "find": self.find.read(cx).text(), "follow": self.follow, "wrap": self.wrap, "time": self.show_time, "pod": self.show_pod })
    }

    fn restore(&mut self, state: &serde_json::Value, cx: &mut Context<Self>) {
        let level = match state["level"].as_str() {
            Some("error") => MinLevel::Error,
            Some("warn") => MinLevel::Warn,
            Some("info") => MinLevel::Info,
            _ => MinLevel::All,
        };
        self.set_min_level(level, cx);
        self.follow = state["follow"].as_bool().unwrap_or(true);
        self.wrap = state["wrap"].as_bool().unwrap_or(true);
        self.show_time = state["time"].as_bool().unwrap_or(true);
        self.show_pod = state["pod"].as_bool().unwrap_or(true);
        if let Some(text) = state["find"].as_str().filter(|t| !t.is_empty()) {
            self.find.update(cx, |f, cx| f.set_text(text, cx));
            self.feed.buffer.set_query(text);
        }
        cx.notify();
    }
}

impl Focusable for ServiceLogsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// `HH:MM:SS` in local time of a server timestamp (epoch ns).
fn clock(at: Option<i64>) -> SharedString {
    let hms = at.and_then(|ns| u64::try_from(ns / 1_000_000).ok()).and_then(devdeck_sys::time::local_hms);
    match hms {
        Some((h, m, s)) => format!("{h:02}:{m:02}:{s:02}").into(),
        None => SharedString::default(),
    }
}

impl Render for ServiceLogsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let c = self.cluster.read(cx);
        let pods_state = match c.pods_state(&self.env, &self.name) {
            Remote::Ready(p) => Remote::Ready(p.len()),
            Remote::Loading => Remote::Loading,
            Remote::Error(e) => Remote::Error(e),
        };
        let count = self.feed.buffer.row_count();
        let toolbar = self.toolbar(cx);
        let empty = |text: String| div().flex_1().flex().items_center().justify_center().text_color(t.mute).child(label(text)).into_any_element();
        let body = if let Some(why) = self.feed.refused() {
            empty(format!("logs: {why}"))
        } else {
            match pods_state {
                Remote::Loading => empty("loading pods…".into()),
                Remote::Error(why) => empty(format!("pods: {why}")),
                Remote::Ready(0) if self.feed.buffer.is_empty() => empty("no pods".into()),
                Remote::Ready(_) if count == 0 => empty(if self.feed.buffer.is_empty() { "no lines yet".into() } else { "no line matches the filter".into() }),
                Remote::Ready(_) => {
                    let (ok, warn, err, mute, dim, fg, current_bg, accent) = (t.ok, t.warn, t.err, t.mute, t.dim, t.fg, t.selection, t.accent);
                    // Exactly what matched: every match tinted, the current one in the accent.
                    let (hit_style, current_style) = (
                        gpui::HighlightStyle { background_color: Some(t.warn.opacity(0.35)), ..Default::default() },
                        gpui::HighlightStyle { background_color: Some(t.accent.opacity(0.7)), color: Some(t.hi), ..Default::default() },
                    );
                    let (row_h, time_w, pod_w) = (t.line_height, t.size(64.), t.size(56.));
                    let (show_time, show_pod, wrap) = (self.show_time, self.show_pod, self.wrap);
                    let cols = if wrap {
                        let cell = devdeck_ui::terminal::cell_size(window, &t);
                        let mut text_w = self.bounds.get().size.width - t.size(28.);
                        if show_time {
                            text_w -= time_w + t.size(8.);
                        }
                        if show_pod {
                            text_w -= pod_w + t.size(8.);
                        }
                        ((text_w / cell.width).floor() as usize).max(20)
                    } else {
                        0
                    };
                    let items = if wrap { self.wrap_rows(cols) } else { count };
                    let current = self.current;
                    let list = scroll_list(
                        "logs-lines",
                        items,
                        cx.processor(move |this, range: std::ops::Range<usize>, _window, _cx| {
                            let b = &this.feed.buffer;
                            let hits = b.hits();
                            let query = b.query();
                            range
                                .filter_map(|i| {
                                    let (row, start) = match &this.wrapped {
                                        Some((_, _, rows)) if wrap => *rows.get(i)?,
                                        _ => (i, 0),
                                    };
                                    Some((b.row_line(row)?, b.row(row)?, start))
                                })
                                .map(|(line, e, start)| {
                                    let color = match (e.note, e.level) {
                                        (true, _) => dim,
                                        (_, Level::Error) => err,
                                        (_, Level::Warn) => warn,
                                        (_, Level::Debug) => mute,
                                        (_, Level::Info) => ok,
                                        (_, Level::Plain) => fg,
                                    };
                                    let first = start == 0;
                                    // The piece shown: the whole line, or its wrapped piece.
                                    let (b0, b1) = if wrap {
                                        let mut ix = e.text.char_indices().map(|(i, _)| i).skip(start);
                                        let b0 = ix.next().unwrap_or(e.text.len());
                                        let b1 = e.text.char_indices().map(|(i, _)| i).nth(start + cols).unwrap_or(e.text.len());
                                        (b0, b1)
                                    } else {
                                        (0, e.text.len())
                                    };
                                    let text: SharedString = if wrap { e.text[b0..b1].to_string().into() } else { e.text.clone() };
                                    let marks: Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> = if hits.binary_search(&line).is_ok() {
                                        match_ranges(&e.text, query)
                                            .into_iter()
                                            .enumerate()
                                            .filter(|(_, r)| r.start < b1 && r.end > b0)
                                            .map(|(nth, r)| {
                                                let style = if current == Some((line, nth)) { current_style } else { hit_style };
                                                (r.start.max(b0) - b0..r.end.min(b1) - b0, style)
                                            })
                                            .collect()
                                    } else {
                                        Vec::new()
                                    };
                                    div()
                                        .h(row_h)
                                        .px_3()
                                        .flex()
                                        .gap_2()
                                        .whitespace_nowrap()
                                        .when(current.is_some_and(|(l, _)| l == line), |d| d.bg(current_bg).border_l_2().border_color(accent))
                                        .when(show_time, |d| d.child(div().flex_none().w(time_w).text_color(mute).child(if first { clock(e.at) } else { SharedString::default() })))
                                        .when(show_pod, |d| d.child(div().flex_none().w(pod_w).overflow_hidden().text_color(mute).child(if first { SharedString::from(short(&e.pod).to_string()) } else { SharedString::default() })))
                                        .child(div().flex_none().text_color(color).child(gpui::StyledText::new(text).with_highlights(marks)))
                                })
                                .collect()
                        }),
                    )
                    .track_scroll(&self.scroll);
                    let list = if wrap {
                        list
                    } else {
                        list.scroll_xy().with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained).with_width_from_item(self.feed.buffer.longest_row())
                    };
                    list.flex_1().min_h_0().into_any_element()
                }
            }
        };

        let bounds = self.bounds.clone();
        div()
            .id("logs-view")
            .key_context("LogsView")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &crate::actions::ToggleFollow, _, cx| this.toggle_follow(cx)))
            .on_action(cx.listener(|this, _: &Find, window, cx| this.find.read(cx).focus_handle(cx).focus(window, cx)))
            .on_action(cx.listener(|this, _: &NextMatch, _, cx| this.step_match(1, cx)))
            .on_action(cx.listener(|this, _: &PrevMatch, _, cx| this.step_match(-1, cx)))
            .on_action(cx.listener(|this, _: &TogglePause, _, cx| this.toggle_pause(cx)))
            .on_action(cx.listener(|this, _: &ToggleWrap, _, cx| this.toggle_wrap(cx)))
            .on_action(cx.listener(|this, _: &OnlyErrors, _, cx| this.set_min_level(MinLevel::Error, cx)))
            .on_action(cx.listener(|this, _: &WarnAndUp, _, cx| this.set_min_level(MinLevel::Warn, cx)))
            .on_action(cx.listener(|this, _: &InfoAndUp, _, cx| this.set_min_level(MinLevel::Info, cx)))
            .on_action(cx.listener(|this, _: &Everything, _, cx| this.set_min_level(MinLevel::All, cx)))
            .on_action(cx.listener(|this, _: &PickContainer, window, cx| this.open_picker(Self::container_menu, None, window, cx)))
            .on_action(cx.listener(|this, _: &PickPod, window, cx| this.open_picker(Self::pod_menu, None, window, cx)))
            .on_action(cx.listener(|this, _: &Options, window, cx| this.toolbar(cx).open_more(window, cx)))
            .on_action(cx.listener(|this, _: &LevelMenu, window, cx| this.toolbar(cx).open_menu_of("logs.level", window, cx)))
            .on_action(cx.listener(|this, _: &ToggleTime, _, cx| {
                this.show_time = !this.show_time;
                this.wrapped = None;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TogglePodColumn, _, cx| {
                this.show_pod = !this.show_pod;
                this.wrapped = None;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PreviousRun, _, cx| this.previous_run(cx)))
            .child(canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().size_full())
            .child(toolbar)
            .child(body)
    }
}
