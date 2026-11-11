//! App view (`app/<id>`, R6.3, R6.5, R6.6, R6.7): header with the env and how it
//! applies, missing tools named, then commands by tier with their run state and
//! actions, and its repository's branch with a way to its git panel; then its notes (R12.3):
//! one alphabetical list, made, opened, renamed and deleted here. Keys (`AppView`): ↑↓
//! select (commands, then notes), ⏎ run (or open a note), s stop, r restart, o output,
//! h hidden, g git, n new note.

use super::{ViewCx, run_mark};
use crate::runner::RunKey;
use devdeck_core::apps::{Command, Tier};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::Scrolls;
use devdeck_ui::{Icon, Theme, ToastKind, action_button, icon, icon_action, label, section, theme, toasts};
use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::*,
};

actions!(app_view, [SelectNext, SelectPrev, RunSelected, StopSelected, RestartSelected, OpenOutput, ToggleHidden, OpenGit, NewNote]);

/// A selected note is `note:<id>` in [`AppView::selected`] (commands are `source:name`).
const NOTE_KEY: &str = "note:";

fn note_of(selected: &str) -> Option<&str> {
    selected.strip_prefix(NOTE_KEY)
}

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
        let notes = ctx.services.read(cx).notes.clone();
        let mut subs = vec![cx.observe(&ctx.services, |_, _, cx| cx.notify()), cx.observe(&notes, |_, _, cx| cx.notify())];
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

    /// A new terminal in its folder, opened as its other views are.
    pub fn open_terminal(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.ctx.services.read(cx).catalog.app(&self.app).map(|a| a.dir.display().to_string()) else { return };
        let Some(n) = self.ctx.deck.upgrade().map(|d| d.read(cx).next_terminal(cx)) else { return };
        self.ctx.open(ViewAddr::Terminal { n, dir }, cx);
    }

    /// Fetches its repository's remotes.
    pub fn fetch(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else { return };
        if let Err(why) = repo.update(cx, |r, cx| r.run(devdeck_core::git::Op::Fetch, "Fetch".into(), "fetching…".into(), cx)) {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("Fetch: {why}"), vec![], cx));
        }
    }

    /// Opens the git panel and asks it to do `op`: its checks and confirmations are the
    /// panel's, the one place git asks.
    pub fn git(&mut self, op: super::git_view::Queued, cx: &mut Context<Self>) {
        self.open_git(cx);
        let Some(root) = self.repo.as_ref().map(|r| r.read(cx).root.display().to_string()) else { return };
        let panel = self.ctx.deck.upgrade().and_then(|d| d.read(cx).view(&ViewAddr::Git { root }.to_ref())).and_then(|v| v.downcast::<super::git_view::GitView>().ok());
        if let Some(panel) = panel {
            panel.update(cx, |p, cx| p.queue(op, cx));
        }
    }

    /// Its repository: the branch, behind and ahead of its upstream, and what can be done
    /// with it. `None` outside a repository.
    fn git_block(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        use super::git_view::Queued;
        let repo = self.repo.as_ref()?.read(cx);
        let snapshot = repo.snapshot();
        let head = snapshot.map(|s| match &s.status.head {
            devdeck_core::git::Head::Branch(b) | devdeck_core::git::Head::Unborn(b) => b.clone(),
            devdeck_core::git::Head::Detached(c) => format!("detached at {c}"),
        });
        let current = snapshot.and_then(|s| s.branches.iter().find(|b| b.current && b.remote.is_none())).cloned();
        let actions = div()
            .flex()
            .items_center()
            .child(icon_action("app-fetch", Icon::Fetch, "fetch", true, t).on_click(cx.listener(|this, _, _, cx| this.fetch(cx))))
            .child(icon_action("app-pull", Icon::Pull, "pull…", true, t).on_click(cx.listener(|this, _, _, cx| this.git(Queued::Pull, cx))))
            .child(icon_action("app-push", Icon::Push, "push…", true, t).on_click(cx.listener(|this, _, _, cx| this.git(Queued::Push, cx))))
            .child(icon_action("app-new-branch", Icon::NewBranch, "new branch…", true, t).on_click(cx.listener(|this, _, _, cx| this.git(Queued::NewBranch, cx))))
            .child(icon_action("app-git-panel", Icon::Git, "branches, changes: the git panel", true, t).on_click(cx.listener(|this, _, _, cx| this.open_git(cx))));
        let branch_row = div()
            .id("app-branch")
            .h(t.row_h)
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(t.radius_s)
            .cursor_pointer()
            .hover(|s| s.bg(t.hover))
            .on_click(cx.listener(|this, _, _, cx| this.open_git(cx)))
            .child(icon(Icon::Git, t.mute, t).size(t.icon_small))
            .child(label(head.unwrap_or_else(|| "reading…".into())).flex_1().min_w_0())
            .child(match current.as_ref().and_then(|b| b.upstream.clone().map(|u| (b, u))) {
                Some((b, upstream)) => div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(devdeck_ui::split_bar("app-ahead-behind", b.behind, b.ahead, t))
                    .child(div().flex_none().text_color(t.dim).child(upstream))
                    .into_any_element(),
                None => div().flex_none().text_color(t.dim).child("no upstream").into_any_element(),
            });
        Some(div().flex().flex_col().child(section("Git", t).flex().items_center().justify_between().child(actions)).child(branch_row))
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

    /// What ↑↓ go through: commands in display order, then notes.
    fn selectable(&self, cx: &App) -> Vec<String> {
        let notes = self.ctx.services.read(cx).notes.read(cx).of(&self.app);
        self.visible_commands(cx).into_iter().map(|c| c.id).chain(notes.into_iter().map(|n| format!("{NOTE_KEY}{}", n.id))).collect()
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.selectable(cx);
        if rows.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|s| rows.iter().position(|r| r == s));
        let next = match at {
            None => 0,
            Some(i) => (i as isize + delta).clamp(0, rows.len() as isize - 1) as usize,
        };
        self.selected = Some(rows[next].clone());
        cx.notify();
    }

    pub fn open_note(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ctx.open(ViewAddr::Note { id: id.to_string() }, cx);
    }

    pub fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        super::note_view::new_note(&self.ctx, &self.app, window, cx);
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

    /// Runs `f` on the selected command (the first when none is); a selected note is
    /// opened instead when `open_note`, else left alone.
    fn with_selected(&mut self, cx: &mut Context<Self>, open_note: bool, f: impl FnOnce(&mut Self, &str, &mut Context<Self>)) {
        if self.selected.is_none() {
            self.move_selection(0, cx);
        }
        let Some(sel) = self.selected.clone() else { return };
        match note_of(&sel) {
            Some(id) if open_note => self.open_note(id, cx),
            Some(_) => {}
            None => f(self, &sel, cx),
        }
    }
}

impl AppView {
    /// The menu of a note row (right-click, shift+F10).
    pub fn note_menu(this: &gpui::Entity<Self>, id: &str, cx: &App) -> Vec<devdeck_ui::MenuItem> {
        let notes = this.read(cx).ctx.services.read(cx).notes.clone();
        let (me, id1, id2, id3, n2, n3) = (this.clone(), id.to_string(), id.to_string(), id.to_string(), notes.clone(), notes);
        vec![
            devdeck_ui::MenuItem::new("Open", move |_, cx| me.update(cx, |v, cx| v.open_note(&id1, cx))),
            devdeck_ui::MenuItem::new("Rename…", move |window, cx| super::note_view::rename_note(&n2, &id2, window, cx)),
            devdeck_ui::MenuItem::new("Delete…", move |window, cx| super::note_view::delete_note(&n3, &id3, window, cx)),
        ]
    }

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

/// "12 min": how long something has run (it runs: no "ago").
fn lasted(d: std::time::Duration) -> String {
    match d.as_secs() {
        0..60 => "<1 min".into(),
        s @ 60..3600 => format!("{} min", s / 60),
        s @ 3600..86400 => format!("{} h", s / 3600),
        s => format!("{} d", s / 86400),
    }
}

/// Every tool its commands need, found or missing; a missing one says what it blocks.
fn tools_block(app: &devdeck_core::apps::App, tools: &devdeck_core::apps::Tools, t: &Theme) -> gpui::Div {
    let mut needed: Vec<&str> = app.commands.iter().flat_map(|c| c.tools.iter().map(String::as_str)).collect();
    needed.sort_unstable();
    needed.dedup();
    let mut block = div().flex().flex_col().child(section(format!("Tools · {}", needed.len()), t));
    for tool in needed {
        let blocked: Vec<&str> = app.commands.iter().filter(|c| c.tools.iter().any(|x| x == tool) && tools.missing(app, c).is_some_and(|m| m.starts_with(tool))).map(|c| c.name()).collect();
        let found = blocked.is_empty();
        block = block.child(
            div()
                .h(t.row_h)
                .px_2()
                .flex()
                .items_center()
                .gap_2()
                .debug_selector(move || format!("tool-{tool}"))
                .child(if found { icon(Icon::Check, t.ok, t).size(t.icon_small) } else { icon(Icon::Warning, t.warn, t).size(t.icon_small) })
                .child(div().flex_none().w(t.size(140.)).truncate().text_color(if found { t.fg } else { t.warn }).child(tool.to_string()))
                .child(label(if found { "found".to_string() } else { format!("not found: {} can't run", blocked.join(", ")) }).flex_1().min_w_0().text_color(t.mute)),
        );
    }
    block
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
        let focused = self.focus.contains_focused(window, cx);

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
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(div().flex_none().text_color(t.hi).child(app.name.clone()))
                    .child(div().flex_none().text_color(t.dim).child(app.kind.clone()))
                    .child(label(app.folder.clone()).text_color(t.dim).flex_1().min_w_0())
                    .child(icon_action("app-terminal", Icon::Terminals, "a terminal in its folder", true, &t).on_click(cx.listener(|this, _, _, cx| this.open_terminal(cx))))
                    .when(self.repo.is_some(), |d| d.child(icon_action("app-git", Icon::Git, "its git panel", true, &t).on_click(cx.listener(|this, _, _, cx| this.open_git(cx))))),
            )
            .child(env_line);

        // Left: what it runs, its repository, its notes. Right: what it needs. Side by side
        // when there's room, else one under the other.
        let mut commands = div().flex().flex_col();
        let now = cx.background_executor().now();
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
            commands = commands.child(head);
            if collapsed {
                continue;
            }
            for (i, c) in cmds.iter().enumerate() {
                let key = RunKey::new(&app.id, &c.id);
                let state = runner.state(&key);
                let refusal = tools.missing(&app, c);
                let running = state.is_some_and(|s| s.is_running());
                let ran = runner.running_for(&key, now).map(lasted);
                let selected = self.selected.as_deref() == Some(c.id.as_str());
                let n = tier as usize * 100 + i;
                let (r1, r2, r3, r4, r5, r6) = (c.id.clone(), c.id.clone(), c.id.clone(), c.id.clone(), c.id.clone(), c.id.clone());
                let row_label: SharedString = c.name().to_string().into();
                let can_run = refusal.is_none() && state != Some(devdeck_core::runs::RunState::Running);
                let run_tip = refusal.clone().map_or_else(|| format!("run {}", c.name()), |r| format!("can't run: {r}"));
                commands = commands.child(
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
                        // How long it has run, while it runs: the dot says it runs.
                        .child(div().flex_none().w(t.size(56.)).truncate().text_color(t.mute).debug_selector(move || format!("cmd-ran-{r6}")).children(ran))
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

        // Notes (R12.3): one alphabetical list.
        let notes = self.ctx.services.read(cx).notes.read(cx).of(&self.app);
        let mut notes_block = div().flex().flex_col().child(
            section(format!("Notes · {}", notes.len()), &t)
                .flex()
                .items_center()
                .justify_between()
                .child(icon_action("new-note", Icon::Plus, "new note", true, &t).debug_selector(|| "new-note".into()).on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx)))),
        );
        for (i, n) in notes.iter().enumerate() {
            let key = format!("{NOTE_KEY}{}", n.id);
            let selected = self.selected.as_deref() == Some(key.as_str());
            let (open, menu_id, sel, selector) = (n.id.clone(), n.id.clone(), key.clone(), n.id.clone());
            notes_block = notes_block.child(
                div()
                    .id(("note", i))
                    .debug_selector(move || format!("note-{selector}"))
                    .h(t.row_h)
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(t.radius_s)
                    .cursor_pointer()
                    .when(selected, |d| d.bg(if focused { t.selection } else { t.hover }))
                    .on_mouse_down(gpui::MouseButton::Right, {
                        let (me, key) = (cx.entity(), key.clone());
                        move |e, window, cx| {
                            me.update(cx, |v, cx| {
                                v.selected = Some(key.clone());
                                cx.notify();
                            });
                            let items = AppView::note_menu(&me, &menu_id, cx);
                            devdeck_ui::open_menu(e.position, items, window, cx);
                        }
                    })
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                        this.selected = Some(sel.clone());
                        this.open_note(&open, cx);
                    }))
                    .child(icon(Icon::Note, t.mute, &t).size(t.icon_small))
                    .child(label(n.name.clone()).flex_1().min_w_0()),
            );
        }

        let left = div()
            .flex_1()
            .min_w(t.size(380.))
            .flex()
            .flex_col()
            .child(commands)
            .when_some(self.git_block(&t, cx), |d, g| d.child(g))
            .child(notes_block);
        let right = div().flex_1().min_w(t.size(300.)).flex().flex_col().child(tools_block(&app, &tools, &t));
        let body = div().id("app-body").flex_1().min_h_0().scroll_y().px_3().pb_2().flex().flex_wrap().gap_x_6().child(left).child(right);

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
            .on_action(cx.listener(|this, _: &RunSelected, _, cx| this.with_selected(cx, true, |s, c, cx| s.run(c, cx))))
            .on_action(cx.listener(|this, _: &StopSelected, _, cx| this.with_selected(cx, false, |s, c, cx| s.stop(c, cx))))
            .on_action(cx.listener(|this, _: &RestartSelected, _, cx| this.with_selected(cx, false, |s, c, cx| s.restart(c, cx))))
            .on_action(cx.listener(|this, _: &OpenOutput, _, cx| this.with_selected(cx, true, |s, c, cx| s.open_output(c, cx))))
            .on_action(cx.listener(|this, _: &crate::actions::ContextMenu, window, cx| {
                if this.selected.is_none() {
                    this.move_selection(0, cx);
                }
                if let Some(sel) = this.selected.clone() {
                    let at = this.origin.get() + gpui::point(theme(cx).size(140.), theme(cx).size(96.));
                    let items = match note_of(&sel) {
                        Some(id) => AppView::note_menu(&cx.entity(), id, cx),
                        None => AppView::command_menu(&cx.entity(), &sel),
                    };
                    devdeck_ui::open_menu(at, items, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NewNote, window, cx| this.new_note(window, cx)))
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

#[cfg(test)]
mod tests {
    use super::lasted;
    use std::time::Duration;

    #[test]
    fn how_long_it_has_run() {
        assert_eq!(lasted(Duration::from_secs(20)), "<1 min");
        assert_eq!(lasted(Duration::from_secs(125)), "2 min");
        assert_eq!(lasted(Duration::from_secs(7300)), "2 h");
        assert_eq!(lasted(Duration::from_secs(3 * 86400)), "3 d");
    }
}
