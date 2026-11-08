//! App view (`app/<id>`, R6.3, R6.5, R6.6, R6.7): header with the env and how it
//! applies, missing tools named, then commands by tier with their run state and
//! actions, and its repository's branch with a way to its git panel. Keys (`AppView`): ↑↓
//! select, ⏎ run, s stop, r restart, o output, h hidden, g git.

use super::{ViewCx, run_mark};
use crate::runner::RunKey;
use devdeck_core::apps::{Command, Tier};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{Icon, ToastKind, action_button, icon, icon_text, label, section, theme, toasts};
use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::*,
};

actions!(app_view, [SelectNext, SelectPrev, RunSelected, StopSelected, RestartSelected, OpenOutput, ToggleHidden, OpenGit]);

pub struct AppView {
    pub app: String,
    ctx: ViewCx,
    focus: FocusHandle,
    pub selected: Option<String>,
    pub show_hidden: bool,
    /// Top-left on screen (keyboard-opened menus are placed from it).
    origin: std::rc::Rc<std::cell::Cell<gpui::Point<gpui::Pixels>>>,
    /// Its repository, when it's in one (read while the view is visible).
    repo: Option<gpui::Entity<crate::repo::Repo>>,
    looking: Option<crate::repo::Looking>,
    _subs: Vec<Subscription>,
}

impl AppView {
    pub fn new(app: String, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let mut subs = vec![cx.observe(&ctx.services, |_, _, cx| cx.notify())];
        let dir = ctx.services.read(cx).catalog.app(&app).map(|a| a.dir.clone());
        let repo = dir.and_then(|d| ctx.services.update(cx, |s, cx| s.repo(&d, cx)));
        if let Some(r) = &repo {
            subs.push(cx.observe(r, |_, _, cx| cx.notify()));
        }
        subs.extend(ctx.on_visibility(cx, |this: &mut Self, visible, cx| {
            this.looking = visible.then(|| this.repo.as_ref().map(|r| r.update(cx, |r, cx| r.glance(cx)))).flatten();
        }));
        Self { app, ctx, focus: cx.focus_handle(), selected: None, show_hidden: false, origin: Default::default(), repo, looking: None, _subs: subs }
    }

    /// Opens its repository's git panel.
    pub fn open_git(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = &self.repo else {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Info, format!("{} isn't in a git repository.", self.app), vec![], cx));
            return;
        };
        let root = repo.read(cx).root.display().to_string();
        self.ctx.open(ViewAddr::Git { root }, cx);
    }

    /// Commands in display order (hidden ones only when shown).
    fn visible_commands(&self, cx: &App) -> Vec<Command> {
        let s = self.ctx.services.read(cx);
        let Some(app) = s.catalog.app(&self.app) else { return Vec::new() };
        Tier::ALL
            .into_iter()
            .filter(|t| *t != Tier::Hidden || self.show_hidden)
            .flat_map(|t| app.tier(t).into_iter().cloned())
            .collect()
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let cmds = self.visible_commands(cx);
        if cmds.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|s| cmds.iter().position(|c| &c.id == s));
        let next = match at {
            None => 0,
            Some(i) => (i as isize + delta).clamp(0, cmds.len() as isize - 1) as usize,
        };
        self.selected = Some(cmds[next].id.clone());
        cx.notify();
    }

    fn key(&self, cmd: &str) -> RunKey {
        RunKey::new(&self.app, cmd)
    }

    /// Runs a command; a refusal (missing tool, already running) becomes a toast.
    pub fn run(&mut self, cmd: &str, cx: &mut Context<Self>) {
        let (runner, env) = {
            let s = self.ctx.services.read(cx);
            (s.runner.clone(), s.env.clone())
        };
        crate::port_scan::start_or_toast(&runner, &self.key(cmd), env, cx);
    }

    pub fn stop(&mut self, cmd: &str, cx: &mut Context<Self>) {
        let runner = self.ctx.services.read(cx).runner.clone();
        let key = self.key(cmd);
        runner.update(cx, |r, cx| r.stop(&key, cx));
    }

    pub fn restart(&mut self, cmd: &str, cx: &mut Context<Self>) {
        let (runner, env) = {
            let s = self.ctx.services.read(cx);
            (s.runner.clone(), s.env.clone())
        };
        let key = self.key(cmd);
        if let Err(why) = runner.update(cx, |r, cx| r.restart(&key, env, cx)) {
            let text = format!("{}: {why}", runner.read(cx).title(&key));
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
        }
    }

    pub fn open_output(&mut self, cmd: &str, cx: &mut Context<Self>) {
        self.ctx.open(ViewAddr::Run { app: self.app.clone(), cmd: cmd.into() }, cx);
    }

    fn with_selected(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Self, &str, &mut Context<Self>)) {
        if self.selected.is_none() {
            self.move_selection(0, cx);
        }
        if let Some(cmd) = self.selected.clone() {
            f(self, &cmd, cx);
        }
    }
}

impl AppView {
    /// The menu of a command row (right-click, shift+F10).
    pub fn command_menu(this: &gpui::Entity<Self>, cmd: &str) -> Vec<devdeck_ui::MenuItem> {
        let item = |label: &'static str, f: fn(&mut Self, &str, &mut Context<Self>)| {
            let (me, cmd) = (this.clone(), cmd.to_string());
            devdeck_ui::MenuItem::new(label, move |_, cx| me.update(cx, |v, cx| f(v, &cmd, cx)))
        };
        vec![
            item("Run", Self::run),
            item("Stop", Self::stop),
            item("Restart", Self::restart),
            item("Output", Self::open_output),
        ]
    }
}

/// The selected command and whether hidden commands are shown.
impl super::KeepsState for AppView {
    fn save(&self, _: &App) -> serde_json::Value {
        serde_json::json!({ "selected": self.selected, "show_hidden": self.show_hidden })
    }

    fn restore(&mut self, state: &serde_json::Value, cx: &mut Context<Self>) {
        self.selected = state["selected"].as_str().map(String::from);
        self.show_hidden = state["show_hidden"].as_bool().unwrap_or(false);
        cx.notify();
    }
}

impl Focusable for AppView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let services = self.ctx.services.read(cx);
        let Some(app) = services.catalog.app(&self.app).cloned() else {
            return div().size_full().into_any_element();
        };
        let runner = services.runner.read(cx);
        let tools = runner.tools().clone();
        let env = services.env.clone();
        let missing = tools.missing_for(&app);
        let focused = self.focus.contains_focused(window, cx);
        let branch = self.repo.as_ref().and_then(|r| r.read(cx).snapshot()).map(|s| match &s.status.head {
            devdeck_core::git::Head::Branch(b) => format!("on {b}"),
            devdeck_core::git::Head::Detached(c) => format!("detached at {c}"),
            devdeck_core::git::Head::Unborn(b) => format!("on {b}"),
        });

        let env_line = match &env {
            Some(e) => div()
                .flex()
                .gap_2()
                .child(label(format!("@{e}")).text_color(if services.protected_env() { t.err } else { t.env }))
                .child(label(format!("· {}", app.env.describe(e))).text_color(t.mute)),
            None => div().child(label(format!("no env · {}", app.env.describe("-"))).text_color(t.mute)),
        };
        let header = div()
            .flex_none()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .border_b_1()
            .border_color(t.line_soft)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .min_w_0()
                    .child(div().flex_none().text_color(t.hi).child(app.name.clone()))
                    .child(div().flex_none().text_color(t.dim).child(app.kind.clone()))
                    .child(label(app.folder.clone()).text_color(t.dim).min_w_0())
                    .when_some(branch, |d, b| d.child(div().flex_none().text_color(t.mute).child(b)))
                    .when(self.repo.is_some(), |d| {
                        d.child(devdeck_ui::button("app-git", "git", &t).tooltip(devdeck_ui::tip("its repository's git panel")).on_click(cx.listener(|this, _, _, cx| this.open_git(cx))))
                    }),
            )
            .child(env_line)
            .children(missing.iter().map(|m| icon_text(Icon::Warning, t.warn, m.clone(), &t)));

        let mut body = div().id("app-commands").flex_1().min_h_0().overflow_y_scroll().px_3().pb_2().flex().flex_col();
        for tier in Tier::ALL {
            let cmds = app.tier(tier);
            if cmds.is_empty() {
                continue;
            }
            let collapsed = tier == Tier::Hidden && !self.show_hidden;
            let heading = format!("{} · {}", tier.title(), cmds.len());
            // Hidden ones fold: open or not, after the heading.
            let heading = section(heading, &t)
                .flex()
                .items_center()
                .gap_1()
                .when(tier == Tier::Hidden, |d| d.child(icon(if collapsed { Icon::Expand } else { Icon::Open }, t.mute, &t)));
            let mut head = div().id(("tier", tier as usize)).child(heading);
            if tier == Tier::Hidden {
                head = head.cursor_pointer().on_click(cx.listener(|this, _, _, cx| {
                    this.show_hidden = !this.show_hidden;
                    cx.notify();
                }));
            }
            body = body.child(head);
            if collapsed {
                continue;
            }
            for (i, c) in cmds.iter().enumerate() {
                let key = RunKey::new(&app.id, &c.id);
                let state = runner.state(&key);
                let refusal = tools.missing(&app, c);
                let running = state.is_some_and(|s| s.is_running());
                let selected = self.selected.as_deref() == Some(c.id.as_str());
                let n = tier as usize * 100 + i;
                let (r1, r2, r3, r4, r5) = (c.id.clone(), c.id.clone(), c.id.clone(), c.id.clone(), c.id.clone());
                let row_label: SharedString = c.name().to_string().into();
                let can_run = refusal.is_none() && state != Some(devdeck_core::runs::RunState::Running);
                let run_tip = refusal.clone().map_or_else(|| format!("run {}", c.name()), |r| format!("can't run: {r}"));
                body = body.child(
                    div()
                        .id(("cmd", n))
                        .debug_selector(move || format!("cmd-{r5}"))
                        .h(t.row_h)
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(t.radius_s)
                        .when(selected, |d| d.bg(if focused { t.selection } else { t.hover }))
                        .on_mouse_down(gpui::MouseButton::Right, {
                            let (me, id) = (cx.entity(), c.id.clone());
                            move |e, window, cx| {
                                me.update(cx, |v, cx| {
                                    v.selected = Some(id.clone());
                                    cx.notify();
                                });
                                devdeck_ui::open_menu(e.position, AppView::command_menu(&me, &id), window, cx);
                            }
                        })
                        .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                            this.selected = Some(r1.clone());
                            if e.click_count() == 2 {
                                this.open_output(&r1, cx);
                            }
                            cx.notify();
                        }))
                        .child(div().flex_none().min_w(t.size(28.)).flex().items_center().child(match refusal {
                            Some(_) => icon(Icon::Warning, t.warn, &t).size(t.icon_small).into_any_element(),
                            None => run_mark(state, ("cmd-state", n), t.icon_small, &t),
                        }))
                        .child(div().flex_none().w(t.size(110.)).truncate().text_color(if refusal.is_some() { t.dim } else { t.fg }).child(row_label))
                        .child(label(c.line.clone()).flex_1().min_w_0().text_color(t.dim))
                        .when(!c.ports.is_empty(), |d| {
                            d.child(div().flex_none().text_color(t.mute).child(c.ports.iter().map(|p| format!(":{p}")).collect::<Vec<_>>().join(" ")))
                        })
                        .when_some(refusal.clone(), |d, r| d.child(div().flex_none().text_color(t.warn).child(r)))
                        .child(action_button(("run", n), Icon::Run, t.icon_small, can_run, run_tip, &t).when(can_run, |d| d.on_click(cx.listener(move |this, _, _, cx| this.run(&r2, cx)))))
                        .child(action_button(("stop", n), Icon::Stop, t.icon_small, running, "stop", &t).when(running, |d| d.on_click(cx.listener(move |this, _, _, cx| this.stop(&r3, cx)))))
                        .child(
                            action_button(("restart", n), Icon::Restart, t.icon_small, refusal.is_none(), "restart", &t)
                                .when(refusal.is_none(), |d| d.on_click(cx.listener(move |this, _, _, cx| this.restart(&r4, cx)))),
                        )
                        .child({
                            let id = c.id.clone();
                            action_button(("output", n), Icon::Output, t.icon_small, true, "output", &t).on_click(cx.listener(move |this, _, _, cx| this.open_output(&id, cx)))
                        }),
                );
            }
        }

        div()
            .id("app-view")
            .key_context("AppView")
            .relative()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &RunSelected, _, cx| this.with_selected(cx, |s, c, cx| s.run(c, cx))))
            .on_action(cx.listener(|this, _: &StopSelected, _, cx| this.with_selected(cx, |s, c, cx| s.stop(c, cx))))
            .on_action(cx.listener(|this, _: &RestartSelected, _, cx| this.with_selected(cx, |s, c, cx| s.restart(c, cx))))
            .on_action(cx.listener(|this, _: &OpenOutput, _, cx| this.with_selected(cx, |s, c, cx| s.open_output(c, cx))))
            .on_action(cx.listener(|this, _: &crate::actions::ContextMenu, window, cx| {
                if this.selected.is_none() {
                    this.move_selection(0, cx);
                }
                if let Some(cmd) = this.selected.clone() {
                    let at = this.origin.get() + gpui::point(theme(cx).size(140.), theme(cx).size(96.));
                    devdeck_ui::open_menu(at, AppView::command_menu(&cx.entity(), &cmd), window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenGit, _, cx| this.open_git(cx)))
            .on_action(cx.listener(|this, _: &ToggleHidden, _, cx| {
                this.show_hidden = !this.show_hidden;
                cx.notify();
            }))
            .child({
                let origin = self.origin.clone();
                gpui::canvas(move |b, _, _| origin.set(b.origin), |_, _, _, _| {}).absolute().size_full()
            })
            .child(header)
            .child(body)
            .into_any_element()
    }
}
