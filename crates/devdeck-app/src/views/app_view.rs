//! App view (`app/<id>`, R6.3, R6.5, R6.6, R6.7): header with the env and how it
//! applies, missing tools named, then commands by tier with their run state and
//! actions, and its repository's branch with a way to its git panel; then its notes (R12.3):
//! one alphabetical list, made, opened, renamed and deleted here. Keys (`AppView`): ↑↓
//! select (commands, then notes), ⏎ run (or open a note), s stop, r restart, o output,
//! h hidden, g git, n new note.

use super::{ViewCx, run_mark};
use crate::runner::RunKey;
use crate::sidebar::Activity;
use devdeck_core::apps::{Command, Tier};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::Scrolls;
use devdeck_ui::{Icon, Theme, ToastKind, action_button, icon, icon_action, label, theme, toasts};
use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::*,
};

actions!(app_view, [SelectNext, SelectPrev, RunSelected, StopSelected, RestartSelected, OpenOutput, ToggleHidden, OpenGit, NewNote, OpenTerminal]);

/// Its header's action (a tab's, the Apps view's details'): the registry's id.
pub const OPEN_TERMINAL: &str = "app_view.terminal";

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
    /// Its repository's git panel, in it (the same view as a `git/<root>` tab; it reads
    /// as this view shows, and opens what it opens over it).
    git: Option<gpui::Entity<super::git_view::GitView>>,
    /// While on screen: redraws now and then, so how long a run has run moves.
    tick: Option<gpui::Task<()>>,
    /// Its scroll, read by its scrollbar.
    scroll: gpui::ScrollHandle,
    _subs: Vec<Subscription>,
}

/// How often a shown app view redraws while one of its commands runs (its "12 min").
pub const RAN_TICK: std::time::Duration = std::time::Duration::from_secs(30);

impl AppView {
    pub fn new(app: String, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let notes = ctx.services.read(cx).notes.clone();
        let mut subs = vec![cx.observe(&ctx.services, |_, _, cx| cx.notify()), cx.observe(&notes, |_, _, cx| cx.notify())];
        let dir = ctx.services.read(cx).catalog.app(&app).map(|a| a.dir.clone());
        let repo = dir.and_then(|d| ctx.services.update(cx, |s, cx| s.repo(&d, cx)));
        if let Some(r) = &repo {
            subs.push(cx.observe(r, |_, _, cx| cx.notify()));
        }
        // Its deployments on every env (like the matrix): the cluster's changes redraw.
        if let Some(cluster) = ctx.services.read(cx).cluster.clone() {
            cluster.update(cx, |c, cx| c.want_all(cx));
            subs.push(cx.observe(&cluster, |_, _, cx| cx.notify()));
        }
        subs.extend(ctx.on_visibility(cx, |this: &mut Self, visible, cx| {
            this.looking = visible.then(|| this.repo.as_ref().map(|r| r.update(cx, |r, cx| r.glance(cx)))).flatten();
            this.tick = visible.then(|| {
                cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(RAN_TICK).await;
                        let Ok(()) = this.update(cx, |v, cx| {
                            let runner = v.ctx.services.read(cx).runner.clone();
                            if runner.read(cx).app_running(&v.app) {
                                cx.notify();
                            }
                        }) else {
                            return;
                        };
                    }
                })
            });
        }));
        let git = repo.as_ref().map(|r| {
            let (root, r, ctx) = (r.read(cx).root.clone(), r.clone(), ctx.clone());
            cx.new(|cx| super::git_view::GitView::new(root, r, ctx, cx))
        });
        Self { app, ctx, focus: cx.focus_handle(), selected: None, show_hidden: false, origin: Default::default(), repo, looking: None, git, tick: None, scroll: gpui::ScrollHandle::new(), _subs: subs }
    }

    /// A new terminal in its folder, opened as its other views are.
    pub fn open_terminal(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.ctx.services.read(cx).catalog.app(&self.app).map(|a| a.dir.display().to_string()) else { return };
        let Some(n) = self.ctx.deck.upgrade().map(|d| d.read(cx).next_terminal(cx)) else { return };
        self.ctx.open(ViewAddr::Terminal { n, dir }, cx);
    }

    /// Where the workspace's cluster runs it, one row per env: the tag, its pods. An app
    /// deployed as nothing of its own shows its project's (a monorepo served as one,
    /// [`devdeck_core::links::project_of`]). A row goes to it in Cluster, where it's
    /// deployed, its logs and history are; envs that can't be read say why. `None` until
    /// a deployment of it is found (none, without a cluster).
    fn deployments_block(&self, t: &Theme, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let Deployed { rows, via, problems } = self.deployed(cx)?;
        // Only once found: an app deployed nowhere has no panel.
        if rows.is_empty() {
            return None;
        }
        let heading = match &via {
            Some(p) if !rows.is_empty() => format!("Deployments · {} · {p}", rows.len()),
            _ => format!("Deployments · {}", rows.len()),
        };
        let mut block = div().flex().flex_col();
        let note = |text: String, color: gpui::Hsla| div().px_2().h(t.row_h).flex().items_center().min_w_0().child(label(text).text_color(color));
        // A table: env, tag, pods, a rule between them.
        let cell = |d: gpui::Div| d.flex_none().h_full().flex().items_center().pr_2().border_r_1().border_color(t.line_soft);
        for (i, (env, d, tag)) in rows.iter().enumerate() {
            let (ctx, name, env_name) = (self.ctx.clone(), d.name.clone(), env.name.clone());
            let (ctx2, name2, env2) = (ctx.clone(), name.clone(), env_name.clone());
            let progress = d.progress.label();
            block = block.child(
                div()
                    .id(("deployment", i))
                    .debug_selector({
                        let env = env_name.clone();
                        move || format!("deployment-{env}")
                    })
                    .h(t.row_h)
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(t.radius_s)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_click(move |_, _, cx| ctx.pivot(Activity::Cluster, name.clone(), Some(env_name.clone()), cx))
                    .child(
                        cell(div().w(t.size(84.)).gap_1())
                            .child(div().min_w_0().truncate().text_color(if env.protected { t.err } else { t.env }).child(format!("@{}", env.name)))
                            .when(env.protected, |r| r.child(icon(Icon::Protected, t.err, t).size(t.icon_small))),
                    )
                    .child(label(tag.clone()).flex_1().min_w_0())
                    .child(devdeck_ui::pods_bar(&format!("pods-{}", env.name), &pod_marks(d), t))
                    .when_some(progress, |r, p| r.child(div().flex_none().text_color(t.warn).child(p.to_string())))
                    .child(
                        devdeck_ui::go_action(("deployment-go", i), Icon::Cluster, format!("{} on {} in Cluster", d.name, env.name), t)
                            .on_click(move |_, _, cx| ctx2.pivot(Activity::Cluster, name2.clone(), Some(env2.clone()), cx)),
                    ),
            );
        }
        for (env, why) in problems {
            block = block.child(note(format!("@{env}: {why}"), t.warn));
        }
        Some(devdeck_ui::panel("app-deployments", heading, Vec::new(), block, t))
    }

    /// Where the cluster runs it ([`AppView::deployments_block`] draws it).
    fn deployed(&self, cx: &App) -> Option<Deployed> {
        let services = self.ctx.services.read(cx);
        let cluster = services.cluster.clone()?;
        let all = services.catalog.apps.clone();
        let c = cluster.read(cx);
        let (mut own, mut parts, mut project) = (Vec::new(), Vec::new(), Vec::new());
        let mut problems = Vec::new();
        let project_name = devdeck_core::links::project_of(&self.app);
        for env in &c.envs {
            let state = c.state(&env.name);
            if let Some(p) = state.and_then(|s| s.problem.clone()) {
                problems.push((env.name.clone(), p));
            }
            let Some(deps) = state.and_then(|s| s.deployments.as_ref()) else { continue };
            for d in deps.values() {
                let image = d.main().image.short_name();
                if devdeck_core::links::app_for(&d.name, image, &all).ok().flatten().as_deref() == Some(self.app.as_str()) {
                    own.push((env.clone(), d.clone(), d.tag()));
                } else if let Some(k) = d.init_containers.iter().find(|k| devdeck_core::links::part_app(&k.name, &k.image.raw, &all).as_deref() == Some(self.app.as_str())) {
                    // Served inside it: its own tag there (its part, deployed on its own).
                    parts.push((env.clone(), d.clone(), k.image.label()));
                } else if project_name.is_some_and(|p| devdeck_core::links::deploys_as(&d.name, image, p)) {
                    project.push((env.clone(), d.clone(), d.tag()));
                }
            }
        }
        let (rows, via) = if !own.is_empty() {
            (own, None)
        } else if let Some((_, d, _)) = parts.first() {
            let via = format!("in {}", d.name);
            (parts, Some(via))
        } else {
            (project, project_name.map(|p| format!("as {p}")))
        };
        Some(Deployed { rows, via, problems })
    }

    /// Terminals in its folder, by number.
    fn terminals_here(&self, app: &devdeck_core::apps::App, cx: &App) -> Vec<u32> {
        let dir = app.dir.display().to_string();
        self.ctx
            .deck
            .upgrade()
            .map(|d| d.read(cx).terminals(cx))
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, tabs)| matches!(ViewAddr::parse(&tabs.view), ViewAddr::Terminal { dir: d, .. } if d == dir))
            .map(|(n, _)| n)
            .collect()
    }

    /// The shells in its folder and what they run; a row goes to it in Terminals.
    fn terminals_block(&self, app: &devdeck_core::apps::App, t: &Theme, cx: &mut Context<Self>) -> gpui::Div {
        let terminals = self.terminals_here(app, cx);
        let title = format!("Terminals · {}", terminals.len());
        let runner = self.ctx.services.read(cx).runner.read(cx);
        let mut block = div().flex().flex_col();
        if terminals.is_empty() {
            block = block.child(div().px_2().h(t.row_h).flex().items_center().text_color(t.dim).child("none in its folder"));
        }
        for n in terminals {
            let key = RunKey::terminal(n);
            let running = runner.state(&key).is_some_and(|s| s.is_running());
            let doing = runner.terminal_program(&key).unwrap_or_else(|| if running { "idle".into() } else { "not running".into() });
            let (ctx, ctx2) = (self.ctx.clone(), self.ctx.clone());
            block = block.child(
                div()
                    .id(("terminal", n as usize))
                    .debug_selector(move || format!("terminal-{n}"))
                    .h(t.row_h)
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(t.radius_s)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_click(move |_, _, cx| ctx.pivot(Activity::Terminals, n.to_string(), None, cx))
                    .child(devdeck_ui::dot(if running { t.ok } else { t.dim }, running, t))
                    .child(div().flex_none().w(t.size(40.)).text_color(t.mute).child(n.to_string()))
                    .child(label(doing).flex_1().min_w_0())
                    .child(devdeck_ui::go_action(("terminal-go", n as usize), Icon::Terminals, format!("terminal {n} in Terminals"), t).on_click(move |_, _, cx| {
                        ctx2.pivot(Activity::Terminals, n.to_string(), None, cx)
                    })),
            );
        }
        devdeck_ui::panel("app-terminals", title, Vec::new(), block, t)
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

/// A deployment's pods as the cluster counts them: the ready ones, then the others,
/// failing when it says why (at most a dozen drawn).
pub(super) fn pod_marks(d: &devdeck_core::cluster::Deployment) -> Vec<devdeck_ui::PodMark> {
    use devdeck_ui::PodMark;
    let wanted = d.replicas.min(12) as usize;
    let ready = (d.ready as usize).min(wanted);
    let other = if d.failure.is_some() { PodMark::Failing } else { PodMark::Starting };
    std::iter::repeat_n(PodMark::Ready, ready).chain(std::iter::repeat_n(other, wanted - ready)).collect()
}

/// "12 min": how long something has run (it runs: no "ago").
pub(super) fn lasted(d: std::time::Duration) -> String {
    match d.as_secs() {
        0..60 => "<1 min".into(),
        s @ 60..3600 => format!("{} min", s / 60),
        s @ 3600..86400 => format!("{} h", s / 3600),
        s => format!("{} d", s / 86400),
    }
}

/// Every tool its commands need, found or missing; a missing one says what it blocks.
/// The tools its commands need and the machine lacks, each with what it blocks; `None`
/// when nothing is missing.
fn tools_block(app: &devdeck_core::apps::App, tools: &devdeck_core::apps::Tools, t: &Theme) -> Option<gpui::Div> {
    let missing: Vec<&str> = needed_tools(app).into_iter().filter(|tool| !blocked_by(app, tools, tool).is_empty()).collect();
    if missing.is_empty() {
        return None;
    }
    let row = || div().h(t.row_h).px_2().flex().items_center().gap_2().min_w_0();
    let title = format!("Missing tools · {}", missing.len());
    let mut block = div().flex().flex_col();
    for tool in missing {
        let what = blocked_by(app, tools, tool);
        block = block.child(
            row()
                .debug_selector(move || format!("tool-{tool}"))
                .child(icon(Icon::Warning, t.warn, t).size(t.icon_small))
                .child(div().flex_none().text_color(t.warn).child(tool.to_string()))
                .child(label(format!("not found: {} can't run", what.join(", "))).flex_1().min_w_0().text_color(t.mute)),
        );
    }
    Some(devdeck_ui::panel("app-tools", title, Vec::new(), block, t))
}

/// Every tool its commands need, alphabetical, once each.
fn needed_tools(app: &devdeck_core::apps::App) -> Vec<&str> {
    let mut needed: Vec<&str> = app.commands.iter().flat_map(|c| c.tools.iter().map(String::as_str)).collect();
    needed.sort_unstable();
    needed.dedup();
    needed
}

/// The commands `tool` missing keeps from running (none: it's there).
fn blocked_by<'a>(app: &'a devdeck_core::apps::App, tools: &devdeck_core::apps::Tools, tool: &str) -> Vec<&'a str> {
    app.commands.iter().filter(|c| c.tools.iter().any(|x| x == tool) && tools.missing(app, c).is_some_and(|m| m.starts_with(tool))).map(|c| c.name()).collect()
}

/// Where the cluster runs an app: per env, its own deployments, else its project's
/// (`via`); the envs that can't be read.
struct Deployed {
    /// Each with the tag it runs there (a part's own, inside the deployment).
    rows: Vec<(devdeck_core::catalog::Env, devdeck_core::cluster::Deployment, String)>,
    via: Option<String>,
    problems: Vec<(String, String)>,
}

/// Its header's icon: a terminal in its folder (its git is in it).
impl super::Live for AppView {
    fn actions(&self, _cx: &App) -> Vec<super::ViewAction> {
        vec![super::ViewAction { id: OPEN_TERMINAL, icon: Icon::Terminals, on: false }]
    }

    fn run(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if id == OPEN_TERMINAL {
            self.open_terminal(cx);
        }
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
        let focused = self.focus.contains_focused(window, cx);

        // Its name is the header's (a tab's, the Apps view's): here, the facts, on one line.
        let facts = div()
            .flex_none()
            .px_3()
            .h(t.row_h)
            .mt_1()
            .flex()
            .items_center()
            .gap_2()
            .min_w_0()
            .debug_selector(|| "app-facts".into())
            .child(div().flex_none().text_color(t.dim).child(app.kind.clone()))
            .child(match &env {
                Some(e) => div().flex_none().text_color(if services.protected_env() { t.err } else { t.env }).child(format!("@{e}")),
                None => div().flex_none().text_color(t.mute).child("no env"),
            })
            .child(div().flex_none().text_color(t.mute).child(app.env.describe(env.as_deref().unwrap_or("-"))))
            .child(label(app.folder.clone()).text_color(t.dim).flex_1().min_w_0());

        // Left: what it runs, its repository, its notes. Right: what it needs. Side by side
        // when there's room, else one under the other.
        let mut commands = div().flex().flex_col();
        // The names' column fits the longest (monospaced: about 0.6 of the text size a
        // character), within reason; a longer one truncates.
        let longest = app.commands.iter().map(|c| c.name().chars().count()).max().unwrap_or(0).clamp(8, 28);
        let name_w = t.text * (0.62 * longest as f32) + t.size(4.);
        let now = cx.background_executor().now();
        for tier in Tier::ALL {
            let cmds = app.tier(tier);
            if cmds.is_empty() {
                continue;
            }
            let collapsed = tier == Tier::Hidden && !self.show_hidden;
            let heading = format!("{} · {}", tier.title(), cmds.len());
            // Hidden ones fold: open or not, after the heading.
            let heading = div()
                .px_2()
                .pt_1()
                .flex()
                .items_center()
                .gap_1()
                .text_size(t.text_small)
                .text_color(t.dim)
                .child(heading)
                .when(tier == Tier::Hidden, |d| d.child(icon(if collapsed { Icon::Expand } else { Icon::Open }, t.dim, &t)));
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
                        // Running: a faint tint, a color only (R9.10).
                        .when(running && !selected, |d| d.bg(t.ok.opacity(0.06)))
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
                        .child(div().flex_none().w(name_w).truncate().text_color(if refusal.is_some() { t.dim } else { t.fg }).child(row_label))
                        // How long it has run, while it runs: the dot says it runs.
                        .child(div().flex_none().w(t.size(56.)).truncate().text_color(t.mute).debug_selector(move || format!("cmd-ran-{r6}")).children(ran))
                        .child(label(c.line.clone()).flex_1().min_w_0().text_color(t.dim))
                        .when(!c.ports.is_empty(), |d| {
                            d.children(c.ports.iter().enumerate().map(|(j, p)| {
                                let (ctx, port) = (self.ctx.clone(), p.to_string());
                                div()
                                    .id(("port", n * 16 + j))
                                    .debug_selector({
                                        let port = port.clone();
                                        move || format!("port-{port}")
                                    })
                                    .flex_none()
                                    .text_color(t.mute)
                                    .cursor_pointer()
                                    .hover(|s| s.text_color(t.accent))
                                    .tooltip(devdeck_ui::tip(format!(":{port} in Ports")))
                                    .child(format!(":{p}"))
                                    .on_click(move |_, _, cx| ctx.pivot(Activity::Ports, port.clone(), None, cx))
                            }))
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
        let notes_title = format!("Notes · {}", notes.len());
        let new_note = icon_action("new-note", Icon::Plus, "new note", true, &t).debug_selector(|| "new-note".into()).on_click(cx.listener(|this, _, window, cx| this.new_note(window, cx)));
        let mut notes_block = div().flex().flex_col();
        if notes.is_empty() {
            notes_block = notes_block.child(div().px_2().h(t.row_h).flex().items_center().text_color(t.dim).child("none yet"));
        }
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

        // Panels on the page's background, the commands wanting the room: 3 to the right
        // column's 2.
        let commands_title = format!("Commands · {}", app.commands.iter().filter(|c| c.tier != Tier::Hidden).count());
        let left = div()
            .flex_grow(3.)
            .flex_basis(gpui::relative(0.))
            .min_w(t.size(380.))
            .flex()
            .flex_col()
            .gap_3()
            .child(devdeck_ui::panel("app-commands", commands_title, Vec::new(), commands, &t))
            .when_some(self.git.clone(), |d, git| {
                // A fixed height: its branch list scrolls in it.
                d.child(devdeck_ui::panel("app-git", "Git", Vec::new(), div().h(t.size(400.)).min_w_0().flex().flex_col().child(git), &t))
            })
            .child(devdeck_ui::panel("app-notes", notes_title, vec![new_note.into_any_element()], notes_block, &t));
        let right = div()
            .flex_grow(2.)
            .flex_basis(gpui::relative(0.))
            .min_w(t.size(300.))
            .flex()
            .flex_col()
            .gap_3()
            .when_some(self.deployments_block(&t, cx), |d, b| d.child(b))
            .child(self.terminals_block(&app, &t, cx))
            .when_some(tools_block(&app, &tools, &t), |d, b| d.child(b));
        let body = div()
            .id("app-body")
            .flex_1()
            .min_h_0()
            .scroll_y()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .child(div().px_3().pt_3().pb_3().flex().flex_wrap().gap_3().child(left).child(right));
        let body = devdeck_ui::with_scrollbar("app-scrollbar", body, &self.scroll).flex_1();

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
            .on_action(cx.listener(|this, _: &OpenTerminal, _, cx| this.open_terminal(cx)))
            .on_action(cx.listener(|this, _: &ToggleHidden, _, cx| {
                this.show_hidden = !this.show_hidden;
                cx.notify();
            }))
            .child({
                let origin = self.origin.clone();
                gpui::canvas(move |b, _, _| origin.set(b.origin), |_, _, _, _| {}).absolute().size_full()
            })
            .child(facts)
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
