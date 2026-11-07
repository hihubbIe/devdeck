//! Run output (`run/<app>/<cmd>`, R6.7): toolbar with the command, env, state and
//! actions, then the run's terminal, wrapping at the pane width (R4.4 exception) and
//! sized to the pane. A run that never started (restored layout) offers to start it.
//! Keys (`RunView`): ⏎ run, s stop, r restart, i types into the terminal; alt+f follow.
//! In the terminal (`Terminal`, focused by a click or i) keys go to the process except
//! alt+… and ctrl+shift+…: ctrl+shift+⏎/x/r run, stop and restart, ctrl+shift+c/v copy
//! and paste, ctrl+shift+esc leaves it. Dragging selects; the wheel scrolls. When several
//! views show one run, the one focused last sizes its terminal. Output redraws at most
//! every [`OUTPUT_FRAME`], except right after typing, so echo shows at once.

use super::{ViewCx, run_glyph};
use crate::runner::{RunKey, RunnerEvent};
use devdeck_core::runs::RunState;
use devdeck_core::terminal::{Mods, TermSize, key_bytes, paste_bytes};
use devdeck_ui::terminal::{self, Frame, Geometry, Scrollback};
use devdeck_ui::{ToastKind, label, theme, tip, toasts, toggle_button};
use gpui::{
    App, ClipboardItem, Context, CursorStyle, DispatchPhase, EntityId, FocusHandle, Focusable, Global, HitboxBehavior, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render, ScrollEvent,
    StatefulInteractiveElement, Styled, Subscription, Window, actions, canvas, div, prelude::*,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Streaming output redraws at most this often (about 30 frames a second).
pub const OUTPUT_FRAME: Duration = Duration::from_millis(33);
/// Output this soon after a key redraws at once (echo).
const TYPING: Duration = Duration::from_millis(500);

actions!(run_view, [RunAgain, Stop, Restart, FocusTerminal, LeaveTerminal]);

/// The view that sizes each run's terminal: the one focused last.
#[derive(Default)]
struct Sizers(HashMap<RunKey, EntityId>);

impl Global for Sizers {}

pub struct RunView {
    pub key: RunKey,
    ctx: ViewCx,
    focus: FocusHandle,
    /// The terminal area: keys go to the process while it has focus.
    term_focus: FocusHandle,
    pub follow: bool,
    scrollback: Scrollback,
    /// The start and size of the run the selection was made in.
    selected_in: Option<(u32, TermSize)>,
    /// Where the terminal was drawn last (mouse, wheel, follow).
    geometry: Rc<Cell<Option<Geometry>>>,
    /// The terminal size this view asked for and the run doesn't have yet.
    size: Rc<Cell<Option<TermSize>>>,
    /// Wheel movement short of a row.
    wheel: f32,
    /// The last redraw for output, a redraw waiting for its turn, the last input.
    drawn: Option<Instant>,
    pending: Option<gpui::Task<()>>,
    typed: Option<Instant>,
    _subs: Vec<Subscription>,
}

impl RunView {
    pub fn new(app: String, cmd: String, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let runner = ctx.services.read(cx).runner.clone();
        let key = RunKey::new(&app, &cmd);
        let me = cx.entity_id();
        let subs = vec![
            cx.observe(&runner, |_, _, cx| cx.notify()),
            cx.subscribe(&runner, |this, _, e: &RunnerEvent, cx| {
                if let RunnerEvent::Output { key } = e
                    && *key == this.key
                {
                    this.output(cx);
                }
            }),
            cx.on_release(move |this, cx| this.give_up_sizing(me, cx)),
        ];
        // A hidden view no longer paints: another view of the run must be able to size it.
        let subs = subs.into_iter().chain(ctx.on_visibility(cx, move |this, visible, cx| {
            if !visible {
                this.give_up_sizing(me, cx);
            }
        }));
        let subs = subs.collect();
        Self {
            key,
            ctx,
            focus: cx.focus_handle(),
            term_focus: cx.focus_handle(),
            follow: true,
            scrollback: Scrollback::default(),
            selected_in: None,
            geometry: Rc::default(),
            size: Rc::default(),
            wheel: 0.,
            drawn: None,
            pending: None,
            typed: None,
            _subs: subs,
        }
    }

    /// New output: redraws now, or once [`OUTPUT_FRAME`] has passed since the last
    /// redraw, so a fast stream costs at most that many frames.
    fn output(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let now = cx.background_executor().now();
        let typing = self.typed.is_some_and(|t| now < t + TYPING);
        match self.drawn.map(|d| d + OUTPUT_FRAME).filter(|due| *due > now && !typing) {
            None => {
                self.drawn = Some(now);
                cx.notify();
            }
            Some(due) => {
                self.pending = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(due - now).await;
                    let _ = this.update(cx, |this, cx| {
                        this.pending = None;
                        this.drawn = Some(cx.background_executor().now());
                        cx.notify();
                    });
                }));
            }
        }
    }

    /// Stops being the view that sizes the run's terminal.
    fn give_up_sizing(&self, me: EntityId, cx: &mut App) {
        if let Some(s) = cx.try_global::<Sizers>()
            && s.0.get(&self.key) == Some(&me)
        {
            cx.global_mut::<Sizers>().0.remove(&self.key);
        }
    }

    pub fn toggle_follow(&mut self, cx: &mut Context<Self>) {
        if self.follow
            && let Some(g) = self.geometry.get()
        {
            self.scrollback.hold(true, g.rows, g.dropped, g.shown);
        }
        self.follow = !self.follow;
        cx.notify();
    }

    /// Gives the keys to the run's process.
    pub fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ctx.services.read(cx).runner.read(cx).run(&self.key).is_some() {
            self.term_focus.focus(window, cx);
        }
    }

    pub fn terminal_focused(&self, window: &Window) -> bool {
        self.term_focus.is_focused(window)
    }

    /// Selects from cell `from` to cell `to` (row, column), as a drag does.
    pub fn select(&mut self, from: (usize, usize), to: (usize, usize), cx: &mut Context<Self>) {
        let dropped = self.ctx.services.read(cx).runner.read(cx).run(&self.key).map_or(0, |r| r.terminal.dropped());
        self.scrollback.press((from.0 as u64 + dropped, from.1));
        self.scrollback.drag((to.0 as u64 + dropped, to.1));
        self.scrollback.release();
        cx.notify();
    }

    /// The selected text, if any.
    pub fn selected_text(&self, cx: &App) -> Option<String> {
        let run = self.ctx.services.read(cx).runner.read(cx).run(&self.key)?;
        let (from, to) = self.scrollback.selection(run.terminal.dropped())?;
        Some(run.terminal.text(from, to))
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
            let text = format!("{}: {why}", runner.read(cx).title(&key));
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
        }
    }

    /// Input for the process; it brings the end of the output back into view.
    fn write(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let runner = self.ctx.services.read(cx).runner.clone();
        runner.update(cx, |r, _| r.write(&self.key, bytes));
        self.typed = Some(cx.background_executor().now());
        if !self.follow {
            self.follow = true;
            cx.notify();
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        // Once the process ended, keys are the run view's again (⏎ runs it, r restarts).
        let Some(modes) = self.ctx.services.read(cx).runner.read(cx).run(&self.key).filter(|r| r.state.is_running()).map(|r| r.terminal.modes()) else { return };
        if let Some(bytes) = terminal::keystroke_bytes(&e.keystroke, modes.app_cursor) {
            cx.stop_propagation();
            self.write(&bytes, cx);
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text(cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        let Some(modes) = self.ctx.services.read(cx).runner.read(cx).run(&self.key).map(|r| r.terminal.modes()) else { return };
        self.write(&paste_bytes(&text, modes.bracketed_paste), cx);
    }

    /// The wheel scrolls the history; on a full-screen program's screen it sends cursor
    /// keys when the program asks for that (alternate scroll mode).
    fn scroll(&mut self, e: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(g) = self.geometry.get() else { return };
        let lines = terminal::wheel_rows(e.delta.pixel_delta(g.cell.height).y, g.cell.height, &mut self.wheel);
        let Some(run) = self.ctx.services.read(cx).runner.read(cx).run(&self.key) else { return };
        let (modes, rows, dropped) = (run.terminal.modes(), run.terminal.rows(), run.terminal.dropped());
        if lines == 0 {
            return;
        }
        if modes.alt_screen {
            if modes.alternate_scroll {
                let key = if lines > 0 { "up" } else { "down" };
                let bytes = key_bytes(key, None, Mods::default(), modes.app_cursor).unwrap_or_default().repeat(lines.unsigned_abs());
                let runner = self.ctx.services.read(cx).runner.clone();
                runner.update(cx, |r, _| r.write(&self.key, &bytes));
            }
            return;
        }
        self.follow = self.scrollback.scroll(self.follow, lines, rows, dropped, g.shown);
        cx.notify();
    }
}

impl Focusable for RunView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RunView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            Some(RunState::Killed(signal)) => format!("killed by signal {signal}"),
            Some(RunState::NotStarted) => "couldn't start".into(),
        };
        let running = state == Some(RunState::Running);
        let follow = self.follow;
        // A selection belongs to one start of the run at one size (rows renumber on a resize);
        // a new start is a new terminal, whose rows count from 0 again.
        let stamp = run.map(|r| (r.runs, r.terminal.size()));
        if stamp != self.selected_in {
            if stamp.map(|s| s.0) != self.selected_in.map(|s| s.0) {
                self.scrollback = Scrollback::default();
            } else {
                self.scrollback.clear();
            }
            self.selected_in = stamp;
        }

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
            .child(div().flex_none().text_color(t.hi).child(runner.title(&self.key)))
            .when_some(env, |d, e| d.child(div().flex_none().text_color(t.env).child(format!("@{e}"))))
            .child(label(line).flex_1().min_w_0().text_color(t.dim))
            .child(div().flex_none().text_color(if state.and_then(|s| s.failed()).is_some() { t.err } else { t.mute }).child(state_text))
            .when_some(refusal.clone(), |d, r| d.child(div().flex_none().text_color(t.warn).child(format!("⚠ {r}"))))
            .child(
                button("start", "▶", !running && refusal.is_none(), refusal.clone().map_or("run".into(), |r| format!("can't run: {r}")))
                    .when(!running && refusal.is_none(), |d| d.on_click(cx.listener(|this, _, _, cx| this.act("run", cx)))),
            )
            .child(button("stop", "■", running, "stop".into()).when(running, |d| d.on_click(cx.listener(|this, _, _, cx| this.act("stop", cx)))))
            .child(
                button("restart", "↻", refusal.is_none(), "restart".into())
                    .when(refusal.is_none(), |d| d.on_click(cx.listener(|this, _, _, cx| this.act("restart", cx)))),
            )
            .child(
                toggle_button("follow", "follow", follow, &t)
                    .tooltip(tip("follow the output"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_follow(cx))),
            );

        let body = if run.is_none() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.mute)
                .child(label(refusal.map_or("Not started. ▶ runs it.".to_string(), |r| format!("Can't run: {r}."))))
                .into_any_element()
        } else {
            let pad = t.size(12.);
            let (runner, key, asked, geometry) = (services.runner.clone(), self.key.clone(), self.size.clone(), self.geometry.clone());
            let (me, view, scrollback) = (cx.entity_id(), cx.entity().downgrade(), self.scrollback.clone());
            let focused = self.focus.contains_focused(window, cx);
            let cursor_on = running && self.term_focus.is_focused(window);
            // One canvas draws the visible rows: no element per row, so a frame of streaming
            // output costs little more than its text.
            let area = canvas(
                move |bounds, window, cx| {
                    let t = theme(cx).clone();
                    let cell = terminal::cell_size(window, &t);
                    terminal::sync_look(&t, cell);
                    let size = terminal::fit(bounds.size, cell);
                    // The view focused last sizes the run's terminal: lines wrap at its width,
                    // and a full-screen program gets its rows.
                    let sizers = &mut cx.default_global::<Sizers>().0;
                    if focused {
                        sizers.insert(key.clone(), me);
                    }
                    let sizes = *sizers.entry(key.clone()).or_insert(me) == me;
                    let has = runner.read(cx).run(&key).map(|r| r.terminal.size());
                    if sizes && has.is_some_and(|s| s != size) {
                        if asked.get() != Some(size) {
                            asked.set(Some(size));
                            let (runner, key) = (runner.clone(), key.clone());
                            cx.defer(move |cx| runner.update(cx, |r, cx| r.resize(&key, size, cx)));
                        }
                    } else {
                        asked.set(None);
                    }

                    let runner = runner.read(cx);
                    let term = &runner.run(&key)?.terminal;
                    let (rows, dropped, shown, cols) = (term.rows(), term.dropped(), size.rows as usize, term.size().cols as usize);
                    let first = scrollback.first(follow, rows, dropped, shown);
                    let g = Geometry { origin: bounds.origin, cell, first: dropped + first as u64, rows, shown, cols, dropped };
                    geometry.set(Some(g));
                    // One more row than fit, partly shown.
                    let visible = first..(first + shown + 1).min(rows);
                    let cursor = (cursor_on && term.modes().show_cursor).then(|| term.cursor()).filter(|c| visible.contains(&c.row));
                    let frame = Frame { rows: visible.clone().map(|i| term.row(i)).collect(), first, cols, selection: scrollback.selection(dropped), cursor };
                    Some((frame, cell, window.insert_hitbox(bounds, HitboxBehavior::Normal), g))
                },
                move |bounds, prepared, window, cx| {
                    let Some((frame, cell, hitbox, g)) = prepared else { return };
                    let t = theme(cx).clone();
                    terminal::paint(frame, bounds, cell, &t, window, cx);
                    window.set_cursor_style(CursorStyle::IBeam, &hitbox);
                    let (down, drag, wheel) = (view.clone(), view.clone(), view.clone());
                    let hit = hitbox.clone();
                    window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && e.button == MouseButton::Left && hit.is_hovered(window) {
                            let _ = down.update(cx, |this, cx| {
                                this.scrollback.press(g.at(e.position));
                                cx.notify();
                            });
                        }
                    });
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble && e.dragging() {
                            let _ = drag.update(cx, |this, cx| {
                                if this.scrollback.drag(g.at(e.position)) {
                                    cx.notify();
                                }
                            });
                        }
                    });
                    window.on_mouse_event(move |e: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                            let _ = view.update(cx, |this, cx| {
                                this.scrollback.release();
                                cx.notify();
                            });
                        }
                    });
                    window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                            let _ = wheel.update(cx, |this, cx| this.scroll(e, cx));
                        }
                    });
                },
            )
            .size_full();
            div()
                .id("run-terminal")
                // Keys go to the process only while there is one.
                .when(state.is_some_and(|s| s.is_running()), |d| d.key_context("Terminal"))
                .track_focus(&self.term_focus)
                .flex_1()
                .min_h_0()
                .px(pad)
                .on_key_down(cx.listener(Self::key_down))
                .on_action(cx.listener(|this, _: &terminal::Copy, _, cx| this.copy(cx)))
                .on_action(cx.listener(|this, _: &terminal::Paste, _, cx| this.paste(cx)))
                .on_action(cx.listener(|this, _: &LeaveTerminal, window, cx| this.focus.focus(window, cx)))
                .child(area)
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
            .on_action(cx.listener(|this, _: &FocusTerminal, window, cx| this.focus_terminal(window, cx)))
            .child(toolbar)
            .child(body)
    }
}
