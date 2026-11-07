//! Service logs (`logs/<env>/<name>`, R7.5): every pod's lines merged with a pod
//! column, level filter, pause, follow, find with n / shift+n. The fake feed writes
//! only while the view is visible; M3 streams real pod logs into the same buffer.
//! Keys (`LogsView`): / find, n next, shift+n previous, p pause, 1–4 level filter.

use super::ViewCx;
use crate::cluster::Cluster;
use devdeck_core::cluster::Pod;
use devdeck_core::runs::{Level, Output};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{InputEvent, TextInput, label, theme, tip, toggle_button};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ListHorizontalSizingBehavior,
    ParentElement, Render, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle,
    Window, actions, div, prelude::*, uniform_list,
};
use std::time::Duration;

/// The fake feed writes one line per pod this often.
pub const LOGS_TICK: Duration = Duration::from_millis(500);
pub const LOGS_CAP: usize = 5000;

actions!(logs_view, [Find, NextMatch, PrevMatch, TogglePause, OnlyErrors, WarnAndUp, InfoAndUp, Everything]);

/// Lowest level shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MinLevel {
    Error,
    Warn,
    Info,
    All,
}

fn shown(level: Level, min: MinLevel) -> bool {
    match min {
        MinLevel::All => true,
        MinLevel::Info => level != Level::Debug,
        MinLevel::Warn => matches!(level, Level::Warn | Level::Error),
        MinLevel::Error => level == Level::Error,
    }
}

pub struct ServiceLogsView {
    pub env: String,
    pub name: String,
    ctx: ViewCx,
    cluster: Entity<Cluster>,
    focus: FocusHandle,
    pub find: Entity<TextInput>,
    pub output: Output,
    pub min_level: MinLevel,
    pub paused: bool,
    pub follow: bool,
    /// Index into the filtered rows of the current match.
    pub current: Option<usize>,
    scroll: UniformListScrollHandle,
    feed: Option<Task<()>>,
    tick: u64,
    _subs: Vec<Subscription>,
}

impl ServiceLogsView {
    pub fn new(env: String, name: String, ctx: ViewCx, cluster: Entity<Cluster>, cx: &mut Context<Self>) -> Self {
        let find = cx.new(|cx| TextInput::new("find", cx));
        let mut subs = vec![
            cx.observe(&cluster, |_, _, cx| cx.notify()),
            cx.subscribe(&find, |this, _, e: &InputEvent, cx| match e {
                InputEvent::Changed(_) => {
                    this.current = None;
                    this.step_match(1, cx);
                }
                InputEvent::Submit(_) => this.step_match(1, cx),
                InputEvent::Cancel => {
                    this.find.update(cx, |f, cx| f.set_text("", cx));
                    this.current = None;
                    cx.notify();
                }
            }),
        ];
        if let Some(deck) = ctx.deck.upgrade() {
            subs.push(cx.observe(&deck, |this, _, cx| this.visibility_changed(cx)));
        }
        cluster.update(cx, |c, cx| {
            c.deployments(&env, cx);
        });
        let me = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = me.update(cx, |this, cx| this.visibility_changed(cx));
        });
        Self {
            env,
            name,
            ctx,
            cluster,
            focus: cx.focus_handle(),
            find,
            output: Output::new(LOGS_CAP),
            min_level: MinLevel::All,
            paused: false,
            follow: true,
            current: None,
            scroll: UniformListScrollHandle::new(),
            feed: None,
            tick: 0,
            _subs: subs,
        }
    }

    pub fn is_streaming(&self) -> bool {
        self.feed.is_some()
    }

    /// The fake feed runs only while the view is on screen.
    fn visibility_changed(&mut self, cx: &mut Context<Self>) {
        let addr = ViewAddr::Logs { env: self.env.clone(), name: self.name.clone() }.to_ref();
        let visible = self.ctx.deck.upgrade().is_some_and(|d| d.read(cx).is_visible(&addr));
        if visible && self.feed.is_none() {
            self.feed = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(LOGS_TICK).await;
                    if this.update(cx, |v, cx| v.write_tick(cx)).is_err() {
                        return;
                    }
                }
            }));
        } else if !visible {
            self.feed = None;
        }
    }

    fn pods(&self, cx: &App) -> Vec<Pod> {
        self.cluster.read(cx).pods(&self.env, &self.name).to_vec()
    }

    fn write_tick(&mut self, cx: &mut Context<Self>) {
        if self.paused {
            return;
        }
        self.tick += 1;
        for (i, p) in self.pods(cx).iter().enumerate() {
            let short: String = p.name.rsplit('-').next().unwrap_or(&p.name).to_string();
            let n = self.tick + i as u64;
            let line = if !p.ready {
                format!("{short}  ERR back-off restarting failed container ({} restarts)", p.restarts)
            } else {
                match n % 7 {
                    0 => format!("{short}  WRN slow query entities.find {}ms", 400 + n % 500),
                    1 => format!("{short}  DBG cache hit entity:{n:08x}"),
                    2 if n.is_multiple_of(3) => format!("{short}  ERR rabbitmq publish failed: channel closed"),
                    3 => format!("{short}  INF POST /v1/{} 201 {}ms", self.name, 10 + n % 40),
                    _ => format!("{short}  INF GET /v1/{}/{n} 200 {}ms", self.name, 5 + n % 30),
                }
            };
            self.output.push(line);
        }
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    /// Indices into `output.lines` of what the filter shows.
    pub fn rows(&self) -> Vec<usize> {
        self.output.lines.iter().enumerate().filter(|(_, l)| shown(l.level, self.min_level)).map(|(i, _)| i).collect()
    }

    /// Rows (positions in [`rows`]) that contain the find text.
    pub fn matches(&self, cx: &App) -> Vec<usize> {
        let q = self.find.read(cx).text().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        self.rows().iter().enumerate().filter(|(_, i)| self.output.lines[**i].text.to_lowercase().contains(&q)).map(|(r, _)| r).collect()
    }

    pub fn step_match(&mut self, dir: isize, cx: &mut Context<Self>) {
        let m = self.matches(cx);
        if m.is_empty() {
            self.current = None;
            cx.notify();
            return;
        }
        let at = self.current.and_then(|c| m.iter().position(|r| *r == c));
        let next = match at {
            None => 0,
            Some(i) => (i as isize + dir).rem_euclid(m.len() as isize) as usize,
        };
        self.current = Some(m[next]);
        self.follow = false;
        self.scroll.scroll_to_item(m[next], ScrollStrategy::Center);
        cx.notify();
    }

    pub fn set_min_level(&mut self, l: MinLevel, cx: &mut Context<Self>) {
        self.min_level = l;
        self.current = None;
        cx.notify();
    }

    pub fn toggle_follow(&mut self, cx: &mut Context<Self>) {
        self.follow = !self.follow;
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }
}

impl Focusable for ServiceLogsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ServiceLogsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let pods = self.pods(cx).len();
        let rows = self.rows();
        let matches = self.matches(cx);
        let count = rows.len();
        let longest = self.output.longest();
        let level_button = |id: &'static str, text: &'static str, l: MinLevel, key: &'static str, cx: &mut Context<Self>| {
            toggle_button(id, text, self.min_level == l, &t)
                .tooltip(tip(format!("show {text} ({key})")))
                .on_click(cx.listener(move |this, _, _, cx| this.set_min_level(l, cx)))
        };
        let toolbar = div()
            .flex_none()
            .h(t.toolbar_h)
            .px_3()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(t.line_soft)
            .child(div().flex_none().text_color(t.hi).child(format!("{} › {}", self.env, self.name)))
            .child(div().flex_none().text_color(t.mute).px_2().child(format!("{pods} pods · {count} lines")))
            .child(level_button("lvl-err", "errors", MinLevel::Error, "1", cx))
            .child(level_button("lvl-warn", "warn+", MinLevel::Warn, "2", cx))
            .child(level_button("lvl-info", "info+", MinLevel::Info, "3", cx))
            .child(level_button("lvl-all", "all", MinLevel::All, "4", cx))
            .child(div().w(t.size(180.)).px_1().debug_selector(|| "logs-find".into()).child(self.find.clone()))
            .child(div().flex_none().text_color(t.mute).child(match (self.current, matches.len()) {
                (_, 0) => String::new(),
                (Some(c), n) => format!("{}/{n}", matches.iter().position(|m| *m == c).map_or(0, |p| p + 1)),
                (None, n) => format!("{n} found"),
            }))
            .child(div().flex_1())
            .child(
                toggle_button("pause", if self.paused { "paused" } else { "pause" }, self.paused, &t)
                    .tooltip(tip("pause the stream (p)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.paused = !this.paused;
                        cx.notify();
                    })),
            )
            .child(toggle_button("follow", "follow", self.follow, &t).tooltip(tip("follow (alt f)")).on_click(cx.listener(|this, _, _, cx| this.toggle_follow(cx))));

        let body = if pods == 0 {
            div().flex_1().flex().items_center().justify_center().text_color(t.mute).child(label("no pods")).into_any_element()
        } else {
            let (ok, warn, err, mute, fg, hit, current_bg, accent) = (t.ok, t.warn, t.err, t.mute, t.fg, t.drop_bg, t.selection, t.accent);
            let row_h = t.line_height;
            let current = self.current;
            let match_set: std::collections::HashSet<usize> = matches.into_iter().collect();
            uniform_list(
                "logs-lines",
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _window, _cx| {
                    let rows = this.rows();
                    range
                        .filter_map(|r| rows.get(r).map(|i| (r, &this.output.lines[*i])))
                        .map(|(r, l)| {
                            let color = match l.level {
                                Level::Error => err,
                                Level::Warn => warn,
                                Level::Debug => mute,
                                Level::Info => ok,
                                Level::Plain => fg,
                            };
                            let text: SharedString = l.text.clone().into();
                            div()
                                .h(row_h)
                                .px_3()
                                .whitespace_nowrap()
                                .text_color(color)
                                .when(match_set.contains(&r), |d| d.bg(hit))
                                .when(current == Some(r), |d| d.bg(current_bg).border_l_2().border_color(accent))
                                .child(text)
                        })
                        .collect()
                }),
            )
            .track_scroll(&self.scroll)
            .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
            .with_width_from_item(Some(longest))
            .flex_1()
            .min_h_0()
            .into_any_element()
        };

        div()
            .id("logs-view")
            .key_context("LogsView")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &Find, window, cx| this.find.read(cx).focus_handle(cx).focus(window, cx)))
            .on_action(cx.listener(|this, _: &NextMatch, _, cx| this.step_match(1, cx)))
            .on_action(cx.listener(|this, _: &PrevMatch, _, cx| this.step_match(-1, cx)))
            .on_action(cx.listener(|this, _: &TogglePause, _, cx| {
                this.paused = !this.paused;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &OnlyErrors, _, cx| this.set_min_level(MinLevel::Error, cx)))
            .on_action(cx.listener(|this, _: &WarnAndUp, _, cx| this.set_min_level(MinLevel::Warn, cx)))
            .on_action(cx.listener(|this, _: &InfoAndUp, _, cx| this.set_min_level(MinLevel::Info, cx)))
            .on_action(cx.listener(|this, _: &Everything, _, cx| this.set_min_level(MinLevel::All, cx)))
            .child(toolbar)
            .child(body)
    }
}
