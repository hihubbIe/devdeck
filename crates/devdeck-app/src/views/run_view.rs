//! Run output (`run/<app>/<cmd>`, R6.7): toolbar with the command line, its arguments,
//! env, state and actions, then the run's terminal, wrapping at the pane width (R4.4
//! exception) and sized to the pane. A run that never started (restored layout) offers
//! to start it.
//! Arguments: the field's menu (a click, or a / ctrl+shift+a) offers the recent ones,
//! none, or new ones typed in the field (⏎ uses them, esc cancels); choosing starts the
//! run with them, or restarts it when it runs. The tab keeps its arguments.
//! Keys (`RunView`): ⏎ run, s stop, r restart, i types into the terminal; alt+f follow.
//! In the terminal (`Terminal`, focused by a press in the output or i) keys go to the
//! process except alt+… and ctrl+shift+…: ctrl+shift+⏎/x/r run, stop and restart,
//! ctrl+shift+v pastes, ctrl+shift+esc leaves it. Copying the selection: ctrl+shift+c
//! anywhere in the tab, and ctrl+c when something is selected, clearing it (otherwise
//! ctrl+c goes to the process). Dragging selects; the wheel scrolls. A right click (or
//! the menu key with a selection) opens the context menu: Paste, and for the selection
//! Copy and Share with (`context_menu`). With shift
//! held over the output, its links are underlined and shift+click opens one in the
//! browser (`http`, `https` and `file` only). When several
//! views show one run, the one focused last sizes its terminal. Output redraws at most
//! every [`OUTPUT_FRAME`], except right after typing, so echo shows at once.

use super::{ViewCx, run_mark};
use crate::context_menu::{self, Content, Origin};
use crate::runner::{RunKey, RunnerEvent};
use devdeck_core::runs::RunState;
use devdeck_core::terminal::{Mods, TermSize, key_bytes, paste_bytes};
use devdeck_ui::terminal::{self, Frame, Geometry, Scrollback};
use devdeck_ui::{Icon, InputEvent, MenuItem, TextInput, Theme, ToastKind, action_button, icon, icon_text, label, open_menu, theme, tip, toasts, toggle_button};
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase, Entity, EntityId, FocusHandle, Focusable, Global, HitboxBehavior, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Render, ScrollEvent,
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

actions!(run_view, [RunAgain, Stop, Restart, FocusTerminal, LeaveTerminal, ChooseArgs]);

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
    /// The trackpad gesture under way: it keeps its axis (R9.6), so a sideways swipe
    /// never scrolls the history.
    gesture: gpui::OngoingScroll,
    /// The last redraw for output, a redraw waiting for its turn, the last input.
    drawn: Option<Instant>,
    pending: Option<gpui::Task<()>>,
    typed: Option<Instant>,
    /// Restored as running: start the run once it can.
    restoring: bool,
    /// New arguments being typed, and the input's subscription.
    args_input: Option<(Entity<TextInput>, Subscription)>,
    /// Where the arguments field was drawn (its menu opens below it).
    args_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Shift is held over the output: its links show.
    links_on: bool,
    _subs: Vec<Subscription>,
}

impl RunView {
    pub fn new(app: String, cmd: String, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let runner = ctx.services.read(cx).runner.clone();
        let key = RunKey::new(&app, &cmd);
        let terminal = key.is_terminal();
        if terminal {
            // Once built: the tools may be known already, then nothing else would start it.
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |v, cx| v.start_if_restoring(cx));
            })
            .detach();
        }
        let me = cx.entity_id();
        let subs = vec![
            cx.observe(&runner, |this, _, cx| {
                this.start_if_restoring(cx);
                cx.notify();
            }),
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
            gesture: gpui::OngoingScroll::default(),
            drawn: None,
            pending: None,
            typed: None,
            // A terminal's shell starts with its tab (opened or restored), once it can.
            restoring: terminal,
            args_input: None,
            args_bounds: Rc::default(),
            links_on: false,
            _subs: subs,
        }
    }

    /// The menu of arguments: the recent ones (the current one checked), none, new ones.
    /// A terminal has none.
    pub fn choose_args(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.key.is_terminal() {
            return;
        }
        let runner = self.ctx.services.read(cx).runner.read(cx);
        let current = runner.args(&self.key).to_string();
        let mut recent = runner.recent_args(&self.key);
        if !current.is_empty() && !recent.contains(&current) {
            recent.insert(0, current.clone());
        }
        let me = cx.entity().downgrade();
        let pick = |args: String| {
            let me = me.clone();
            move |_: &mut Window, cx: &mut App| {
                let _ = me.update(cx, |v, cx| v.use_args(&args, cx));
            }
        };
        let mut items: Vec<MenuItem> = recent.iter().map(|a| MenuItem::new(a.clone(), pick(a.clone())).checked(*a == current)).collect();
        if !items.is_empty() {
            items.push(MenuItem::separator());
        }
        items.push(MenuItem::new("no arguments", pick(String::new())).checked(current.is_empty()));
        let typing = me.clone();
        items.push(MenuItem::new("new arguments…", move |window, cx| {
            let _ = typing.update(cx, |v, cx| v.type_args(window, cx));
        }));
        let at = self.args_bounds.get().map_or_else(|| window.bounds().center(), |b| b.bottom_left());
        open_menu(at, items, window, cx);
    }

    /// Shows the field as an input with the current arguments, focused.
    pub fn type_args(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.key.is_terminal() {
            return;
        }
        let current = self.ctx.services.read(cx).runner.read(cx).args(&self.key).to_string();
        let input = cx.new(|cx| {
            let mut input = TextInput::new("arguments", cx);
            input.set_text(&current, cx);
            input
        });
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::Submit(text) => {
                let text = text.clone();
                this.close_args_input(window, cx);
                this.use_args(&text, cx);
            }
            InputEvent::Cancel => this.close_args_input(window, cx),
            InputEvent::Changed(_) => {}
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.args_input = Some((input, sub));
        cx.notify();
    }

    fn close_args_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.args_input = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// The arguments field: the input while new ones are typed, else the current ones
    /// (`args…` without) and a ▾, opening the menu. It shrinks before the actions do, so
    /// they stay in a narrow pane.
    fn args_field(&self, args: String, t: &Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        if let Some((input, _)) = &self.args_input {
            let field = div().w(t.size(220.)).flex_shrink(1.).min_w(t.size(80.)).debug_selector(|| "run-args-input".into());
            return field.child(input.clone()).into_any_element();
        }
        let bounds = self.args_bounds.clone();
        let (hover, empty) = (t.hover, args.is_empty());
        div()
            .id("args")
            .debug_selector(|| "run-args".into())
            .relative()
            .flex_shrink(1.)
            .min_w(t.size(60.))
            .max_w(t.size(220.))
            .h(t.button_h)
            .px_2()
            .flex()
            .items_center()
            .gap_1()
            .rounded(t.radius_s)
            .border_1()
            .border_color(if empty { t.line } else { t.accent })
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(tip(if empty { "arguments: recent ones, none, or new ones".to_string() } else { format!("arguments: {args}") }))
            .on_click(cx.listener(|this, _, window, cx| this.choose_args(window, cx)))
            .child(label(if empty { "args…".to_string() } else { args }).text_color(if empty { t.dim } else { t.fg }))
            .child(icon(Icon::Open, t.mute, t))
            .child(canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {}).absolute().size_full())
            .into_any_element()
    }

    /// Whether the output's links show (shift held).
    pub fn links_shown(&self) -> bool {
        self.links_on
    }

    /// The middle of cell `col` of row `row` (numbered as `Terminal::row`), where it was
    /// last drawn; `None` when it isn't shown.
    pub fn cell_center(&self, row: usize, col: usize) -> Option<gpui::Point<Pixels>> {
        let g = self.geometry.get()?;
        let on_screen = (row as u64 + g.dropped).checked_sub(g.first)?;
        let x = g.origin.x + g.cell.width * (col as f32 + 0.5);
        Some(gpui::point(x, g.origin.y + g.cell.height * (on_screen as f32 + 0.5)))
    }

    /// Whether row `row` (numbered as `Terminal::row`) is on screen.
    fn row_shown(&self, row: usize) -> bool {
        self.geometry.get().is_some_and(|g| (row as u64 + g.dropped).checked_sub(g.first).is_some_and(|r| r < g.shown as u64))
    }

    /// Shows or hides the output's links (shift held or released).
    pub fn show_links(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.links_on != on {
            self.links_on = on;
            cx.notify();
        }
    }

    /// Runs with `args`: a restart when it runs, a start otherwise.
    pub fn use_args(&mut self, args: &str, cx: &mut Context<Self>) {
        if self.key.is_terminal() {
            return;
        }
        let runner = self.ctx.services.read(cx).runner.clone();
        runner.update(cx, |r, cx| r.set_args(&self.key, args, cx));
        let running = runner.read(cx).state(&self.key).is_some_and(RunState::is_running);
        self.act(if running { "restart" } else { "run" }, cx);
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

    /// A run restored as running, or a terminal's shell with its tab, starts as soon as it
    /// can (the tools may still be being checked); if it can't run any more, it stays not
    /// started. A terminal the runner already ran (running, or ended by `exit`) isn't
    /// started again by its tab: only ⏎ does that.
    fn start_if_restoring(&mut self, cx: &mut Context<Self>) {
        if !self.restoring {
            return;
        }
        let runner = self.ctx.services.read(cx).runner.read(cx);
        if self.key.is_terminal() && runner.run(&self.key).is_some() {
            self.restoring = false;
            return;
        }
        if runner.tools().pending {
            return;
        }
        match runner.refusal(&self.key) {
            Some(_) => self.restoring = false,
            None => {
                self.restoring = false;
                self.act("run", cx);
            }
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

    /// Drops the selection.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.scrollback.clear();
        cx.notify();
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
        if what == "run" {
            crate::port_scan::start_or_toast(&runner, &key, env, cx);
            return;
        }
        let result = runner.update(cx, |r, cx| match what {
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
        // ctrl+c copies a selection rather than interrupting the process, and clears it:
        // the next ctrl+c interrupts.
        let m = &e.keystroke.modifiers;
        if e.keystroke.key == "c" && m.control && !m.shift && !m.alt && !m.platform && self.selected_text(cx).is_some() {
            cx.stop_propagation();
            self.copy(cx);
            self.scrollback.clear();
            cx.notify();
            return;
        }
        // Once the process ended, keys are the run view's again (⏎ runs it, r restarts).
        let Some(modes) = self.ctx.services.read(cx).runner.read(cx).run(&self.key).filter(|r| r.state.is_running()).map(|r| r.terminal.modes()) else { return };
        if let Some(bytes) = terminal::keystroke_bytes(&e.keystroke, modes.app_cursor) {
            cx.stop_propagation();
            self.write(&bytes, cx);
        }
    }

    /// The right-click menu at `at`: Paste while the process runs, and for a selection
    /// what takes text (Copy, Share with). Returns whether it opened.
    pub fn context_menu(&mut self, at: gpui::Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(deck) = self.ctx.deck.upgrade() else { return false };
        let l = deck.read(cx).layout();
        let Some(here) = l.find_view(&self.ctx.view).and_then(|(p, _)| l.window_of(p).ok()) else { return false };
        let origin = Origin { deck: self.ctx.deck.clone(), services: self.ctx.services.clone(), window: here, source: Some(self.key.clone()) };
        let running = self.ctx.services.read(cx).runner.read(cx).state(&self.key).is_some_and(|s| s.is_running());
        let me = cx.entity().downgrade();
        let own = if running {
            vec![MenuItem::new("Paste", move |_, cx| {
                let _ = me.update(cx, |v, cx| v.paste(cx));
            })]
        } else {
            Vec::new()
        };
        context_menu::open(at, own, self.selected_text(cx).map(Content::Text), &origin, window, cx)
    }

    /// The menu key: the context menu at the end of the selection; without one the tab
    /// menu opens as before.
    fn menu_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let end = self.ctx.services.read(cx).runner.read(cx).run(&self.key).and_then(|r| self.scrollback.selection(r.terminal.dropped())).map(|(_, end)| end);
        // Its end where shown, else the output's top left (it scrolled away).
        let top_left = self.geometry.get().map(|g| gpui::point(g.origin.x + g.cell.width, g.origin.y + g.cell.height));
        let at = end.map(|(row, col)| self.cell_center(row, col).filter(|_| self.row_shown(row)).or(top_left));
        match at.flatten() {
            Some(at) if self.context_menu(at, window, cx) => {}
            _ => cx.propagate(),
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
        let mut delta = e.delta.pixel_delta(g.cell.height);
        if e.delta.precise() {
            self.gesture.filter(&mut delta, e.touch_phase);
        }
        let lines = terminal::wheel_rows(delta.y, g.cell.height, &mut self.wheel);
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

/// Follow, the arguments, and whether the run was running: it starts again in its tab
/// on reopen, with them.
impl super::KeepsState for RunView {
    fn save(&self, cx: &App) -> serde_json::Value {
        let runner = self.ctx.services.read(cx).runner.read(cx);
        let running = runner.was_running(&self.key);
        serde_json::json!({ "follow": self.follow, "running": running, "args": runner.args(&self.key) })
    }

    fn restore(&mut self, state: &serde_json::Value, cx: &mut Context<Self>) {
        self.follow = state["follow"].as_bool().unwrap_or(true);
        if self.key.is_terminal() {
            return;
        }
        if let Some(args) = state["args"].as_str().filter(|a| !a.is_empty()) {
            let runner = self.ctx.services.read(cx).runner.clone();
            runner.update(cx, |r, cx| r.set_args(&self.key, args, cx));
        }
        self.restoring = state["running"].as_bool().unwrap_or(false);
        self.start_if_restoring(cx);
    }
}

/// A terminal's tab gives its keys to the shell; a run's to its toolbar keys first.
impl Focusable for RunView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        if self.key.is_terminal() { self.term_focus.clone() } else { self.focus.clone() }
    }
}

impl Render for RunView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let args = self.ctx.services.read(cx).runner.read(cx).args(&self.key).to_string();
        let args_field = self.args_field(args, &t, cx);
        let services = self.ctx.services.read(cx);
        let runner = services.runner.read(cx);
        let run = runner.run(&self.key);
        let state = run.map(|r| r.state);
        let refusal = runner.refusal(&self.key).filter(|r| r != "already running");
        let env = run.and_then(|r| r.env.clone()).or(services.env.clone());
        let line = runner.line(&self.key, env.as_deref()).unwrap_or_default();
        let state_text = match state {
            None => "not started".to_string(),
            Some(RunState::Running) => "running".into(),
            Some(RunState::Stopping) => "stopping…".into(),
            Some(RunState::Stopped) => "stopped".into(),
            Some(RunState::Exited(0)) => run.and_then(|r| r.took).map_or("succeeded".into(), |d| format!("succeeded in {}", devdeck_core::runs::took(d))),
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

        let button = |id: &'static str, i: Icon, enabled: bool, tooltip: String| {
            action_button(id, i, t.icon, enabled, tooltip, &t).debug_selector(move || format!("run-{id}"))
        };
        // What the run is takes the room left and shrinks (the line first, then the
        // arguments and the name; the state stays whole); the actions always stay visible.
        let info = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap_2()
            .child(run_mark(state, "run-state", t.icon, &t))
            .child(div().id("title").min_w_0().tooltip(tip(runner.title(&self.key))).child(label(runner.title(&self.key)).text_color(t.hi)))
            .when_some(env.filter(|_| !self.key.is_terminal()), |d, e| d.child(div().flex_none().text_color(t.env).child(format!("@{e}"))))
            .when(!self.key.is_terminal(), |d| {
                d.child(div().id("line").flex_1().min_w_0().tooltip(tip(line.clone())).child(label(line).text_color(t.dim))).child(args_field)
            })
            .when(self.key.is_terminal(), |d| d.child(div().flex_1()))
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .text_color(if state.and_then(|s| s.failed()).is_some() { t.err } else { t.mute })
                    .child(state_text),
            )
            .when_some(refusal.clone(), |d, r| d.child(icon_text(Icon::Warning, t.warn, r, &t)));
        let toolbar = div()
            .flex_none()
            .h(t.toolbar_h)
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(t.line_soft)
            .child(info)
            .child(
                button("start", Icon::Run, !running && refusal.is_none(), refusal.clone().map_or("run".into(), |r| format!("can't run: {r}")))
                    .when(!running && refusal.is_none(), |d| d.on_click(cx.listener(|this, _, _, cx| this.act("run", cx)))),
            )
            .child(button("stop", Icon::Stop, running, "stop".into()).when(running, |d| d.on_click(cx.listener(|this, _, _, cx| this.act("stop", cx)))))
            .child(
                button("restart", Icon::Restart, refusal.is_none(), "restart".into())
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
                .gap_1()
                .child(match refusal {
                    // "Not started. ▶ runs it.", with the button's icon.
                    None => div().flex().items_center().gap_1().child("Not started.").child(icon(Icon::Run, t.mute, &t)).child("runs it.").into_any_element(),
                    Some(r) => label(format!("Can't run: {r}.")).into_any_element(),
                })
                .into_any_element()
        } else {
            let pad = t.size(12.);
            let (runner, key, asked, geometry) = (services.runner.clone(), self.key.clone(), self.size.clone(), self.geometry.clone());
            let (me, view, scrollback) = (cx.entity_id(), cx.entity().downgrade(), self.scrollback.clone());
            let focused = self.focus.contains_focused(window, cx);
            let cursor_on = running && self.term_focus.is_focused(window);
            let links_on = self.links_on;
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
                    let links = if links_on { term.links(visible.clone()) } else { Vec::new() };
                    let spans = links.iter().map(|l| (l.from, l.to)).collect();
                    let frame = Frame { rows: visible.clone().map(|i| term.row(i)).collect(), first, cols, selection: scrollback.selection(dropped), cursor, links: spans };
                    let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                    // A hand over each link's cells, row by row.
                    let link_boxes: Vec<(gpui::Hitbox, String)> = links
                        .iter()
                        .flat_map(|l| {
                            (l.from.0.max(first)..=l.to.0).filter(|r| visible.contains(r)).map(move |r| {
                                let (a, b) = (if r == l.from.0 { l.from.1 } else { 0 }, if r == l.to.0 { l.to.1 + 1 } else { cols });
                                let at = gpui::point(bounds.origin.x + cell.width * a as f32, bounds.origin.y + cell.height * (r - first) as f32);
                                (Bounds::new(at, gpui::size(cell.width * b.saturating_sub(a) as f32, cell.height)), l.url.clone())
                            })
                        })
                        .map(|(b, url)| (window.insert_hitbox(b, HitboxBehavior::Normal), url))
                        .collect();
                    Some((frame, cell, hitbox, g, link_boxes))
                },
                move |bounds, prepared, window, cx| {
                    let Some((frame, cell, hitbox, g, link_boxes)) = prepared else { return };
                    let t = theme(cx).clone();
                    terminal::paint(frame, bounds, cell, &t, window, cx);
                    window.set_cursor_style(CursorStyle::IBeam, &hitbox);
                    for (b, _) in &link_boxes {
                        window.set_cursor_style(CursorStyle::PointingHand, b);
                    }
                    let (down, drag, wheel, shift, menu) = (view.clone(), view.clone(), view.clone(), view.clone(), view.clone());
                    let (hit, over, menu_hit) = (hitbox.clone(), hitbox.clone(), hitbox.clone());
                    // A right click: the context menu (the selection stays).
                    window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && e.button == MouseButton::Right && menu_hit.is_hovered(window) {
                            let _ = menu.update(cx, |this, cx| {
                                if this.context_menu(e.position, window, cx) {
                                    cx.stop_propagation();
                                }
                            });
                        }
                    });
                    // Shift held or released while the mouse moves over the output.
                    window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                        let on = e.modifiers.shift && over.is_hovered(window);
                        if phase == DispatchPhase::Bubble && on != links_on {
                            let _ = shift.update(cx, |this, cx| this.show_links(on, cx));
                        }
                    });
                    window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble || e.button != MouseButton::Left {
                            return;
                        }
                        if e.modifiers.shift
                            && let Some((_, url)) = link_boxes.iter().find(|(b, _)| b.is_hovered(window))
                        {
                            cx.open_url(url);
                            cx.stop_propagation();
                            return;
                        }
                        if hit.is_hovered(window) {
                            let _ = down.update(cx, |this, cx| {
                                this.scrollback.press(g.at(e.position));
                                // A press in the output is where keys go next: copy included.
                                this.term_focus.focus(window, cx);
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
            .on_modifiers_changed(cx.listener(|this, e: &gpui::ModifiersChangedEvent, _, cx| this.show_links(e.modifiers.shift, cx)))
            .on_action(cx.listener(|this, _: &crate::actions::ToggleFollow, _, cx| this.toggle_follow(cx)))
            .on_action(cx.listener(|this, _: &terminal::Copy, _, cx| this.copy(cx)))
            .on_action(cx.listener(|this, _: &crate::actions::ContextMenu, window, cx| this.menu_key(window, cx)))
            .on_action(cx.listener(|this, _: &RunAgain, _, cx| this.act("run", cx)))
            .on_action(cx.listener(|this, _: &Stop, _, cx| this.act("stop", cx)))
            .on_action(cx.listener(|this, _: &Restart, _, cx| this.act("restart", cx)))
            .on_action(cx.listener(|this, _: &ChooseArgs, window, cx| this.choose_args(window, cx)))
            .on_action(cx.listener(|this, _: &FocusTerminal, window, cx| this.focus_terminal(window, cx)))
            .child(toolbar)
            .child(body)
    }
}
