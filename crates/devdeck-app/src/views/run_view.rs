//! Run output (`run/<app>/<cmd>`, R6.7): toolbar with the command, env, state and
//! actions, then the output, colored by level, never wrapping (plain lines until M2's
//! terminal grid). A run that never started (restored layout) offers to start it.
//! Keys (`RunView`): ⏎ run, s stop, r restart; alt+f follow.

use super::{ViewCx, run_glyph};
use crate::runner::{RunKey, RunnerEvent};
use devdeck_core::runs::{Level, RunState};
use devdeck_ui::{ToastKind, label, theme, tip, toasts, toggle_button};
use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, ListHorizontalSizingBehavior, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle, Window, actions, div, prelude::*,
    uniform_list,
};

actions!(run_view, [RunAgain, Stop, Restart]);

pub struct RunView {
    pub key: RunKey,
    ctx: ViewCx,
    focus: FocusHandle,
    pub follow: bool,
    scroll: UniformListScrollHandle,
    seen: usize,
    _subs: Vec<Subscription>,
}

impl RunView {
    pub fn new(app: String, cmd: String, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let runner = ctx.services.read(cx).runner.clone();
        let key = RunKey::new(&app, &cmd);
        let subs = vec![
            cx.observe(&runner, |this, _, cx| this.changed(cx)),
            cx.subscribe(&runner, |this, _, e: &RunnerEvent, cx| {
                if let RunnerEvent::Output { key } = e
                    && *key == this.key
                {
                    this.changed(cx);
                }
            }),
        ];
        Self { key, ctx, focus: cx.focus_handle(), follow: true, scroll: UniformListScrollHandle::new(), seen: 0, _subs: subs }
    }

    /// Re-renders only when this run changed, and keeps the end in view while following.
    fn changed(&mut self, cx: &mut Context<Self>) {
        let n = self.ctx.services.read(cx).runner.read(cx).run(&self.key).map_or(0, |r| r.output.lines.len() + r.output.dropped);
        if n != self.seen {
            self.seen = n;
            if self.follow {
                self.scroll.scroll_to_bottom();
            }
        }
        cx.notify();
    }

    pub fn toggle_follow(&mut self, cx: &mut Context<Self>) {
        self.follow = !self.follow;
        if self.follow {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    fn act(&mut self, what: &'static str, cx: &mut Context<Self>) {
        let (runner, env) = {
            let s = self.ctx.services.read(cx);
            (s.runner.clone(), s.env.clone())
        };
        let key = self.key.clone();
        let result = runner.update(cx, |r, cx| match what {
            "run" => r.start(&key, env, cx),
            "restart" => r.restart(&key, env, cx),
            _ => {
                r.stop(&key, cx);
                Ok(())
            }
        });
        if let Err(why) = result {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("{}: {why}", key.title()), vec![], cx));
        }
    }
}

impl Focusable for RunView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RunView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let services = self.ctx.services.read(cx);
        let runner = services.runner.read(cx);
        let run = runner.run(&self.key);
        let state = run.map(|r| r.state);
        let (glyph, color) = run_glyph(state, &t);
        let refusal = runner.refusal(&self.key).filter(|r| r != "already running");
        let line = services.catalog.app(&self.key.app).and_then(|a| a.command(&self.key.cmd)).map(|c| c.line.clone()).unwrap_or_default();
        let env = run.and_then(|r| r.env.clone()).or(services.env.clone());
        let state_text = match state {
            None => "not started".to_string(),
            Some(RunState::Running) => "running".into(),
            Some(RunState::Stopping) => "stopping…".into(),
            Some(RunState::Stopped) => "stopped".into(),
            Some(RunState::Exited(code)) => format!("exited {code}"),
        };
        let running = state == Some(RunState::Running);
        let count = run.map_or(0, |r| r.output.lines.len());
        let longest = run.map_or(0, |r| r.output.longest());
        let follow = self.follow;

        let button = |id: &'static str, glyph: &'static str, enabled: bool, tooltip: String| {
            let hover = t.hover;
            div()
                .id(id)
                .debug_selector(move || format!("run-{id}"))
                .px_2()
                .h(t.button_h)
                .flex()
                .items_center()
                .rounded(t.radius_s)
                .text_color(if enabled { t.fg } else { t.dim })
                .when(enabled, |d| d.hover(move |s| s.bg(hover)).cursor_pointer())
                .tooltip(tip(tooltip))
                .child(glyph)
        };
        let toolbar = div()
            .flex_none()
            .h(t.toolbar_h)
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(t.line_soft)
            .child(div().flex_none().text_color(color).child(glyph))
            .child(div().flex_none().text_color(t.hi).child(self.key.title()))
            .when_some(env, |d, e| d.child(div().flex_none().text_color(t.env).child(format!("@{e}"))))
            .child(label(line).flex_1().min_w_0().text_color(t.dim))
            .child(div().flex_none().text_color(if state.and_then(|s| s.failed()).is_some() { t.err } else { t.mute }).child(state_text))
            .when_some(refusal.clone(), |d, r| d.child(div().flex_none().text_color(t.warn).child(format!("⚠ {r}"))))
            .child(
                button("start", "▶", !running && refusal.is_none(), refusal.clone().map_or("run (⏎)".into(), |r| format!("can't run: {r}")))
                    .when(!running && refusal.is_none(), |d| d.on_click(cx.listener(|this, _, _, cx| this.act("run", cx)))),
            )
            .child(button("stop", "■", running, "stop (s)".into()).when(running, |d| d.on_click(cx.listener(|this, _, _, cx| this.act("stop", cx)))))
            .child(
                button("restart", "↻", refusal.is_none(), "restart (r)".into())
                    .when(refusal.is_none(), |d| d.on_click(cx.listener(|this, _, _, cx| this.act("restart", cx)))),
            )
            .child(
                toggle_button("follow", "follow", follow, &t)
                    .tooltip(tip("follow the output (alt f)"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_follow(cx))),
            );

        let body = if run.is_none() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.mute)
                .child(label(refusal.map_or("Not started. ⏎ or ▶ runs it.".to_string(), |r| format!("Can't run: {r}."))))
                .into_any_element()
        } else {
            let (ok, warn, err, mute, fg) = (t.ok, t.warn, t.err, t.mute, t.fg);
            let row_h = t.line_height;
            uniform_list(
                "run-lines",
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let services = this.ctx.services.read(cx);
                    let runner = services.runner.read(cx);
                    let Some(run) = runner.run(&this.key) else { return Vec::new() };
                    range
                        .filter_map(|i| run.output.lines.get(i))
                        .map(|l| {
                            let color = match l.level {
                                Level::Error => err,
                                Level::Warn => warn,
                                Level::Debug => mute,
                                Level::Info => ok,
                                Level::Plain => fg,
                            };
                            let text: SharedString = l.text.clone().into();
                            div().h(row_h).px_3().whitespace_nowrap().text_color(color).child(text)
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
            .id("run-view")
            .key_context("RunView")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &crate::actions::ToggleFollow, _, cx| this.toggle_follow(cx)))
            .on_action(cx.listener(|this, _: &RunAgain, _, cx| this.act("run", cx)))
            .on_action(cx.listener(|this, _: &Stop, _, cx| this.act("stop", cx)))
            .on_action(cx.listener(|this, _: &Restart, _, cx| this.act("restart", cx)))
            .child(toolbar)
            .child(body)
    }
}
