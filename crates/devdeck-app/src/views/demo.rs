//! Demo views from the M0 spike (logs stream, cluster feed, notes, gallery). Not part
//! of the product; kept to exercise the pane engine and for the manual test.

use crate::kube_feed::{self, KubeState};
use devdeck_ui::{label, theme};
use devdeck_core::layout::ViewRef;
use gpui::{
    AnyView, App, AppContext, Context, FocusHandle, Focusable, IntoElement, ListHorizontalSizingBehavior, ParentElement, Render, SharedString,
    Styled, Task, UniformListScrollHandle, Window, div, prelude::*, uniform_list,
};
use std::time::Duration;

pub fn build(r: &ViewRef, ctx: Option<&super::ViewCx>, cx: &mut App) -> super::Built {
    let s = r.as_str();
    let plain = |view: AnyView| super::Built { view, focus: None, save: None, missing: false };
    if s == "demo/logs" {
        let v = cx.new(|cx| LogsView::new(ctx, cx));
        super::focusable(v, cx)
    } else if s == "demo/deployments" {
        plain(cx.new(DeploymentsView::new).into())
    } else if s == "demo/gallery" {
        plain(cx.new(crate::gallery::Gallery::new).into())
    } else {
        let title: SharedString = title(r).into();
        plain(cx.new(|_| TextView::new(title)).into())
    }
}

/// Tab labels of the demo views.
pub fn title(r: &ViewRef) -> String {
    match r.as_str() {
        "demo/logs" => "dev35 › entityservice · logs".into(),
        "demo/deployments" => "dev35 › deployments".into(),
        "demo/notes" => "notes".into(),
        "demo/gallery" => "components".into(),
        "demo/long" => "a tab whose title is far too long to ever fit in a tab strip".into(),
        other => other.rsplit('/').next().unwrap_or(other).to_string(),
    }
}

// ------------------------------------------------------------------ logs

/// Streams synthetic log lines (2 000/s up to 50 000) into a virtualized list.
/// Stands in for the real terminal grid, which M2 builds on alacritty_terminal.
pub struct LogsView {
    lines: Vec<SharedString>,
    longest: usize,
    follow: bool,
    scroll: UniformListScrollHandle,
    /// The stream; `None` while hidden and once it finished.
    feed: Option<Task<()>>,
    focus: FocusHandle,
    _visibility: Option<gpui::Subscription>,
}

impl Focusable for LogsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

const LOG_TARGET: usize = 50_000;
const LOG_BATCH: usize = 100;
const LOG_TICK: Duration = Duration::from_millis(50);

impl LogsView {
    /// Streams only while on screen (the engine's visibility), like every live view;
    /// without a workspace (bare decks) it streams at once.
    fn new(ctx: Option<&super::ViewCx>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            lines: Vec::new(),
            longest: 0,
            follow: true,
            scroll: UniformListScrollHandle::new(),
            feed: None,
            focus: cx.focus_handle(),
            _visibility: None,
        };
        match ctx {
            Some(ctx) => this._visibility = ctx.on_visibility(cx, |v, visible, cx| v.set_streaming(visible, cx)),
            None => this.set_streaming(true, cx),
        }
        this
    }

    fn set_streaming(&mut self, on: bool, cx: &mut Context<Self>) {
        if !on {
            self.feed = None;
        } else if self.feed.is_none() && self.lines.len() < LOG_TARGET {
            self.feed = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(LOG_TICK).await;
                    let done = this
                        .update(cx, |v, cx| {
                            v.push_batch();
                            let done = v.lines.len() >= LOG_TARGET;
                            if done {
                                v.feed = None;
                            }
                            cx.notify();
                            done
                        })
                        .unwrap_or(true);
                    if done {
                        break;
                    }
                }
            }));
        }
    }

    fn push_batch(&mut self) {
        const MSGS: [(&str, &str); 6] = [
            ("INF", "GET /v1/entities/9DC5RDSJT6 200 12ms"),
            ("INF", "GET /v1/entities/search?q=vessel 200 48ms"),
            ("DBG", "cache hit entity:5b478bb4d82bee0015bb667f"),
            ("WRN", "slow query entities.find 812ms filter={\"type\":\"vessel\",\"tenant\":\"9DC5RDSJT6\"} this line is long on purpose so the view has to scroll horizontally instead of wrapping"),
            ("ERR", "rabbitmq publish failed: channel closed, retrying in 2s"),
            ("INF", "POST /v1/entities 201 31ms"),
        ];
        for _ in 0..LOG_BATCH {
            let n = self.lines.len();
            let (lvl, msg) = MSGS[n % MSGS.len()];
            let line: SharedString = format!("{:>6}  08:41:{:02}.{:03}  {lvl}  {msg}", n + 1, (n / 1000) % 60, n % 1000).into();
            if line.len() > self.lines.get(self.longest).map_or(0, |l| l.len()) {
                self.longest = n;
            }
            self.lines.push(line);
        }
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
    }

    pub fn toggle_follow(&mut self, cx: &mut Context<Self>) {
        self.follow = !self.follow;
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }
}

impl Render for LogsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let (mute, ok, warn, err, accent, line_soft) = (t.mute, t.ok, t.warn, t.err, t.accent, t.line_soft);
        let (row_h, toolbar_h) = (t.line_height, t.toolbar_h);
        let count = self.lines.len();
        let follow = self.follow;
        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &crate::actions::ToggleFollow, _, cx| this.toggle_follow(cx)))
            .size_full()
            .flex()
            .flex_col()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .flex_none()
                    .h(toolbar_h)
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(line_soft)
                    .text_color(mute)
                    .child(format!("{count} lines"))
                    .child(
                        div()
                            .id("follow")
                            .cursor_pointer()
                            .text_color(if follow { accent } else { mute })
                            .child(if follow { "● following" } else { "○ follow" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_follow(cx))),
                    ),
            )
            .child(
                uniform_list(
                    "log-lines",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _window, _cx| {
                        range
                            .map(|i| {
                                let line = this.lines[i].clone();
                                let color = match line.get(22..25) {
                                    Some("ERR") => err,
                                    Some("WRN") => warn,
                                    Some("DBG") => mute,
                                    _ => ok,
                                };
                                div().h(row_h).px_3().whitespace_nowrap().text_color(color).child(line)
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.scroll)
                .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                .with_width_from_item(Some(self.longest))
                .flex_1()
                .min_h_0(),
            )
    }
}

// ------------------------------------------------------------------ text

/// Static text. Lines never wrap; the view scrolls horizontally (R3.9, R4.4).
pub struct TextView {
    title: SharedString,
}

impl TextView {
    fn new(title: SharedString) -> Self {
        Self { title }
    }
}

impl Render for TextView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let lines = [
            format!("# {}", self.title),
            String::new(),
            "Panes are generic containers; any view can be dropped anywhere.".into(),
            "Drag a tab onto another tab strip to move it, onto a pane edge to split.".into(),
            String::new(),
            format!("This last line is deliberately very long so it must scroll horizontally rather than wrap: {}", "lorem-free filler · ".repeat(20)),
        ];
        div()
            .id("text-scroll")
            .size_full()
            .overflow_scroll()
            .p_3()
            .text_color(t.fg)
            .children(lines.into_iter().map(|l| div().whitespace_nowrap().child(l)))
    }
}

// ------------------------------------------------------------------ deployments

/// Read-only deployment list fed by a tokio thread over a channel. Proves the
/// background runtime doesn't keep the UI awake (R1.3, R7.8).
pub struct DeploymentsView {
    state: KubeState,
    _feed: Option<Task<()>>,
}

impl DeploymentsView {
    fn new(cx: &mut Context<Self>) -> Self {
        let Some(rx) = kube_feed::start() else {
            return Self { state: KubeState::NotConfigured, _feed: None };
        };
        let feed = cx.spawn(async move |this, cx| {
            while let Ok(state) = rx.recv().await {
                if this.update(cx, |v, cx| {
                    v.state = state;
                    cx.notify();
                })
                .is_err()
                {
                    break;
                }
            }
        });
        Self { state: KubeState::Loading, _feed: Some(feed) }
    }
}

impl Render for DeploymentsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let body = div().size_full().p_3().flex().flex_col().text_color(t.fg);
        match &self.state {
            KubeState::NotConfigured => body.text_color(t.mute).child(
                label("Cluster not configured. Set DEVDECK_KUBECONFIG, DEVDECK_KUBE_CONTEXT and DEVDECK_KUBE_NAMESPACE.")
                    .debug_selector(|| "kube-message".into()),
            ),
            KubeState::Loading => body.text_color(t.mute).child(label("◐ loading deployments…")),
            KubeState::Error(e) => body.text_color(t.err).child(label(format!("✖ {e}"))),
            KubeState::Ready(rows) => {
                let (name_w, ready_w, row_h) = (t.size(300.), t.size(80.), t.row_h);
                body
                .child(
                    div()
                        .flex()
                        .text_color(t.mute)
                        .child(div().w(name_w).flex_none().child("deployment"))
                        .child(div().w(ready_w).flex_none().child("ready"))
                        .child(div().child("image tag")),
                )
                .children(rows.iter().map(|r| {
                    div()
                        .flex()
                        .h(row_h)
                        .child(div().w(name_w).flex_none().truncate().child(r.name.clone()))
                        .child(
                            div()
                                .w(ready_w)
                                .flex_none()
                                .text_color(if r.ready == r.desired { t.ok } else { t.err })
                                .child(format!("{}/{}", r.ready, r.desired)),
                        )
                        .child(div().truncate().text_color(t.mute).child(r.tag.clone()))
                }))
            }
        }
    }
}

