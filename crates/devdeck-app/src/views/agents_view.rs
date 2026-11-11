//! The agents' overview (`agents`, R6.14): what the Agents full view shows while no
//! session is selected, also a tab. One page of panels: Now (how many sessions do what,
//! what they cost, how busy they were the last hour, minute by minute), Usage (each
//! agent's limits as it reported them, and when they reset), Waiting on you (what each
//! asks, a way there), Sessions (by project: each one's state, context used, cost,
//! changes, its last hour, when it was last active; a row goes there) and Latest answers.

use super::ViewCx;
use crate::agents::{Agents, show_session};
use devdeck_core::agents::{HISTORY_MINUTES, Status};
use devdeck_ui::{Column, Icon, Scrolls, activity_bars, dot, label, table_head, table_row, theme, tip};
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Window, div, prelude::*,
};

const SESSION_COLS: [Column; 7] = [
    Column::fixed("STATE", 84.),
    Column::rest("SESSION"),
    Column::fixed("CONTEXT", 128.),
    Column::fixed("COST", 64.).right(),
    Column::fixed("CHANGES", 92.).right(),
    Column::fixed("LAST HOUR", 244.),
    Column::fixed("ACTIVE", 64.).right(),
];

pub struct AgentsView {
    ctx: ViewCx,
    focus: FocusHandle,
    scroll: gpui::ScrollHandle,
    _subs: Vec<Subscription>,
}

impl AgentsView {
    pub fn new(ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let agents = ctx.services.read(cx).agents.clone();
        let subs = vec![cx.observe(&agents, |_, _, cx| cx.notify())];
        Self { ctx, focus: cx.focus_handle(), scroll: gpui::ScrollHandle::new(), _subs: subs }
    }

    fn agents(&self, cx: &App) -> Entity<Agents> {
        self.ctx.services.read(cx).agents.clone()
    }
}

impl Focusable for AgentsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// A state's color and whether its dot is filled.
fn look(status: Status, t: &devdeck_ui::Theme) -> (gpui::Hsla, bool) {
    match status {
        Status::Working => (t.ok, true),
        Status::NeedsYou => (t.warn, true),
        Status::Done => (t.accent, true),
        Status::Ready => (t.ok, false),
        Status::Off => (t.dim, false),
    }
}

/// Dollars, cents shown.
fn dollars(c: f64) -> String {
    format!("${c:.2}")
}

/// Minutes and seconds worked, said short.
fn worked(secs: f32) -> String {
    match secs as u64 {
        0 => "idle".into(),
        s if s < 60 => format!("{s} s"),
        s => format!("{} min", s / 60),
    }
}

impl Render for AgentsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let agents_entity = self.agents(cx);
        let agents = agents_entity.read(cx);
        let workspace = self.ctx.services.read(cx).workspace.clone();
        let kinds = crate::environment::agent_kinds(cx);
        let root = div().id("agents-view").debug_selector(|| "agents-view".into()).track_focus(&self.focus).size_full().flex().flex_col();
        let sessions = agents.sessions();
        if sessions.is_empty() {
            let hint = if kinds.is_empty() { "No agent is configured: add one to machine.toml ([agents.<id>])." } else { "No session yet: \"<agent> here\" on a tab, a pane or an app, or new session." };
            return root.child(div().flex_1().flex().items_center().justify_center().text_color(t.dim).child(label(hint)));
        }
        let now_s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let mut rows: Vec<_> = sessions.iter().map(|s| (agents.label(&s.id, cx), s.clone(), agents.status(&s.id, cx), agents.history(&s.id))).collect();
        // By project, then by name: a project's sessions together.
        let project = |s: &devdeck_core::agents::AgentSession| s.project.as_deref().map_or_else(|| devdeck_core::config::shown(&s.dir), |p| agents.project_name(p).0);
        rows.sort_by_key(|(label, s, ..)| (s.project.is_none(), project(s).to_lowercase(), label.to_lowercase()));

        // ---- Now: counts by state, cost, and the last hour of all of them together.
        let count = |st: Status| rows.iter().filter(|(_, _, s, _)| *s == st).count();
        let cost: f64 = rows.iter().filter_map(|(_, s, ..)| s.facts.cost).sum();
        let mut all = vec![0.0f32; HISTORY_MINUTES];
        for (_, _, _, h) in &rows {
            for (a, s) in all.iter_mut().zip(h.shares()) {
                *a += s;
            }
        }
        let total: f32 = rows.iter().map(|(.., h)| h.total()).sum();
        let peak = all.iter().cloned().fold(0.0f32, f32::max);
        let busiest = if peak > 0.0 { format!(", {peak:.1} sessions at once at the most") } else { String::new() };
        let together: Vec<f32> = all.iter().map(|a| if peak > 0.0 { a / peak.max(1.0) } else { 0.0 }).collect();
        let mut tally = div().flex().items_center().gap_4().px_2().py_1().min_w_0();
        for st in [Status::Working, Status::NeedsYou, Status::Done, Status::Ready, Status::Off] {
            let n = count(st);
            let (color, filled) = look(st, &t);
            let text = if n == 0 { t.dim } else { t.fg };
            tally = tally.child(
                div().debug_selector(move || format!("agents-count-{}", st.label())).flex().items_center().gap_1().text_color(text).child(dot(color, filled, &t)).child(format!("{n} {}", st.label())),
            );
        }
        if cost > 0.0 {
            tally = tally.child(div().text_color(t.dim).child(format!("{} spent", dollars(cost))));
        }
        let strip = div()
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .py_1()
            .child(activity_bars("agents-all", &together, format!("{} worked in the last hour{busiest}", worked(total)), &t))
            .child(div().text_color(t.dim).text_size(t.text_small).child(format!("last hour · {} worked", worked(total))));
        let now_panel = devdeck_ui::panel("agents-now", "Now", Vec::new(), div().flex().flex_col().min_w_0().child(tally).child(strip), &t);

        // ---- Usage: each agent's limits, as it reported them last.
        let mut limits: Vec<AnyElement> = Vec::new();
        for (agent, list) in agents.limits() {
            let name = kinds.iter().find(|k| &k.id == agent).map_or_else(|| agent.clone(), |k| k.name.clone());
            for (i, l) in list.iter().enumerate() {
                let resets = l.resets.map(|r| format!("resets in {}", super::fmt_age(r.saturating_sub(now_s)))).unwrap_or_default();
                let selector = format!("agents-limit-{agent}-{i}");
                limits.push(
                    div()
                        .h(t.row_h)
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_3()
                        .min_w_0()
                        .child(label(format!("{name} · {}", l.name)).w(t.size(180.)).flex_none())
                        .child(devdeck_ui::usage_bar(&selector, l.used.round() as u32, Some(80), Some(100), &t))
                        .child(div().w(t.size(44.)).flex_none().text_color(if l.used >= 80.0 { t.warn } else { t.fg }).child(format!("{:.0}%", l.used)))
                        .child(label(resets).text_color(t.dim))
                        .into_any_element(),
                );
            }
        }
        let usage = (!limits.is_empty()).then(|| devdeck_ui::panel("agents-usage", "Usage", Vec::new(), div().flex().flex_col().min_w_0().children(limits), &t));

        // ---- Waiting on you: what each asks, a way there.
        let waiting: Vec<AnyElement> = rows
            .iter()
            .filter(|(_, _, st, _)| *st == Status::NeedsYou)
            .map(|(name, s, ..)| {
                let (ws, id) = (workspace.clone(), s.id.clone());
                let asks = s.facts.asks.clone().map_or_else(|| "waits for you".to_string(), |a| format!("asks to use {a}"));
                div()
                    .id(SharedString::from(format!("agents-waiting-{}", s.id)))
                    .h(t.row_h)
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .rounded(t.radius_s)
                    .hover(|d| d.bg(t.hover))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| show_session(&ws, &id, cx))
                    .child(dot(t.warn, true, &t))
                    .child(label(name.clone()).flex_none().max_w(t.size(260.)))
                    .child(label(asks).flex_1().text_color(t.dim))
                    .child(devdeck_ui::icon(Icon::GoCorner, t.dim, &t))
                    .into_any_element()
            })
            .collect();
        let waiting = (!waiting.is_empty()).then(|| devdeck_ui::panel("agents-waiting", format!("Waiting on you · {}", waiting.len()), Vec::new(), div().flex().flex_col().min_w_0().children(waiting), &t));

        // ---- Sessions: a row each, going there.
        let mut table = div().flex().flex_col().min_w_0().child(table_head(&SESSION_COLS, &t));
        for (name, s, st, h) in &rows {
            let (color, filled) = look(*st, &t);
            let state = div().flex().items_center().gap_1().min_w_0().child(dot(color, filled, &t)).child(label(st.label()).text_color(if *st == Status::Off { t.dim } else { t.fg }));
            let agent = kinds.iter().find(|k| k.id == s.agent).map_or_else(|| s.agent.clone(), |k| k.name.clone());
            let session = div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(label(name.clone()).flex_none().max_w(relative_half()))
                .child(label(format!("{} · {agent}", project(s))).text_color(t.dim));
            let context = match s.facts.context {
                Some(c) => div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(devdeck_ui::usage_bar(&format!("agents-context-{}", s.id), c.round() as u32, Some(80), Some(100), &t).w(t.size(72.)))
                    .child(div().text_color(if c >= 80.0 { t.warn } else { t.fg }).child(format!("{c:.0}%")))
                    .into_any_element(),
                None => div().text_color(t.dim).child("–").into_any_element(),
            };
            let cost = s.facts.cost.map_or_else(|| "–".into(), dollars);
            let changes = match (s.facts.added, s.facts.removed) {
                (None, None) => div().text_color(t.dim).child("–").into_any_element(),
                (a, r) => div().flex().gap_1().child(div().text_color(t.ok).child(format!("+{}", a.unwrap_or(0)))).child(div().text_color(t.err).child(format!("−{}", r.unwrap_or(0)))).into_any_element(),
            };
            let tip_text = format!("{} worked in the last hour", worked(h.total()));
            let hour = activity_bars(&format!("agents-hour-{}", s.id), &h.shares(), tip_text, &t).into_any_element();
            let active = super::fmt_age(now_s.saturating_sub(s.active));
            let (ws, id) = (workspace.clone(), s.id.clone());
            let hint = s.facts.prompt.clone().map(|p| format!("last prompt: {p}"));
            table = table.child(
                table_row(
                    SharedString::from(format!("agents-row-{}", s.id)),
                    &SESSION_COLS,
                    vec![state.into_any_element(), session.into_any_element(), context, label(cost).into_any_element(), changes, hour, label(active).text_color(t.dim).into_any_element()],
                    &t,
                )
                .hover(|d| d.bg(t.hover))
                .cursor_pointer()
                .when_some(hint, |d, h| d.tooltip(tip(h)))
                .on_click(move |_, _, cx| show_session(&ws, &id, cx)),
            );
        }
        let sessions_panel = devdeck_ui::panel("agents-sessions", format!("Sessions · {}", rows.len()), Vec::new(), table, &t);

        // ---- Latest answers: the most recently active sessions' last words.
        let mut latest: Vec<_> = rows.iter().filter(|(_, s, ..)| s.facts.message.is_some()).collect();
        latest.sort_by_key(|(_, s, ..)| std::cmp::Reverse(s.active));
        let answers: Vec<AnyElement> = latest
            .into_iter()
            .take(6)
            .map(|(name, s, st, _)| {
                let (color, filled) = look(*st, &t);
                let (ws, id) = (workspace.clone(), s.id.clone());
                div()
                    .id(SharedString::from(format!("agents-answer-{}", s.id)))
                    .h(t.row_h)
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .rounded(t.radius_s)
                    .hover(|d| d.bg(t.hover))
                    .cursor_pointer()
                    .tooltip(tip(s.facts.message.clone().unwrap_or_default()))
                    .on_click(move |_, _, cx| show_session(&ws, &id, cx))
                    .child(dot(color, filled, &t))
                    .child(label(name.clone()).flex_none().max_w(t.size(220.)))
                    .child(label(s.facts.message.clone().unwrap_or_default().replace('\n', " ")).flex_1().text_color(t.dim))
                    .into_any_element()
            })
            .collect();
        let answers = (!answers.is_empty()).then(|| devdeck_ui::panel("agents-answers", "Latest answers", Vec::new(), div().flex().flex_col().min_w_0().children(answers), &t));

        let body = div()
            .id("agents-body")
            .flex_1()
            .min_h_0()
            .scroll_y()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .child(div().p_3().flex().flex_col().gap_3().child(now_panel).children(waiting).children(usage).child(sessions_panel).children(answers));
        root.child(devdeck_ui::with_scrollbar("agents-scrollbar", body, &self.scroll).flex_1())
    }
}

/// Half the room: a session's name leaves the rest to its agent and folder.
fn relative_half() -> gpui::DefiniteLength {
    gpui::relative(0.5)
}
