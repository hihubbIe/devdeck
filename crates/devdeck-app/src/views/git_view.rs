//! Git panel (`git/<repo root>`, R6.10): a repository's branch, upstream and changes, its
//! local and remote branches with their last commit and the envs running them, and the
//! few operations devdeck makes, each asked first in plain words (fetch doesn't ask: it
//! changes neither branches nor files). An operation under way in the repository (a
//! rebase…) is shown, and only its abort is offered. Push never loses anything: it
//! publishes, sends, or replaces a rewritten branch only when the remote's commits are in
//! its history; anything else is refused (see `devdeck_core::git::PushPlan`).
//!
//! Keys (`GitView`): ↑↓ select, ⏎ switch, f fetch, p pull, shift-p push, n new branch,
//! r refresh. Opened from an app's or a service's view; one panel per repository.

use super::ViewCx;
use crate::cluster::Cluster;
use crate::repo::{Looking, Repo};
use devdeck_core::cluster::Remote;
use devdeck_core::git::{Branch, ChangeKind, Doing, Head, Op, PushPlan};
use devdeck_ui::{
    Icon, InputEvent, ModalFocus, TextInput, Theme, ToastKind, button, icon, icon_text, label, modal_frame, pop_modal, push_modal, section, theme,
    tip, toasts,
};
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle, WeakFocusHandle, Window, actions, div, prelude::*,
    uniform_list,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

actions!(git_view, [SelectNext, SelectPrev, SwitchSelected, Fetch, Pull, Push, NewBranch, Refresh, ConfirmGit]);

/// How many changed files are listed before "and N more".
const CHANGES_SHOWN: usize = 8;

pub struct GitView {
    pub root: PathBuf,
    ctx: ViewCx,
    pub repo: Entity<Repo>,
    looking: Option<Looking>,
    focus: FocusHandle,
    filter: Entity<TextInput>,
    pub selected: Option<usize>,
    scroll: UniformListScrollHandle,
    /// A push plan asked for by this panel.
    wants_plan: bool,
    /// Tags running on each env (deployments of this repository's apps): the env chips.
    envs_by_tag: BTreeMap<String, Vec<(String, bool)>>,
    window: Option<AnyWindowHandle>,
    _subs: Vec<Subscription>,
}

/// A branch row: a local branch, or a remote one without a local branch of its name.
#[derive(Clone, Debug)]
pub struct Row {
    pub branch: Branch,
    /// The name it switches to (a remote's without its remote).
    pub local: String,
}

impl GitView {
    pub fn new(root: PathBuf, repo: Entity<Repo>, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| TextInput::new("filter branches", cx));
        // Its buttons say whether edit mode allows them.
        let edit_mode = crate::edit_mode::edit_mode(cx);
        let mut subs = vec![
            cx.observe(&edit_mode, |_, _, cx| cx.notify()),
            cx.observe(&repo, |this, _, cx| this.repo_changed(cx)),
            cx.subscribe(&filter, |this, _, e: &InputEvent, cx| match e {
                InputEvent::Changed(_) => {
                    this.selected = None;
                    cx.notify();
                }
                InputEvent::Submit(_) => this.switch_selected(cx),
                InputEvent::Cancel => {}
            }),
            cx.observe(&ctx.services, |this, _, cx| this.follow_cluster(cx)),
        ];
        subs.extend(ctx.on_visibility(cx, |this, visible, cx| {
            this.looking = visible.then(|| this.repo.update(cx, |r, cx| r.look(cx)));
        }));
        let mut this = Self {
            root,
            ctx,
            repo,
            looking: None,
            focus: cx.focus_handle(),
            filter,
            selected: None,
            scroll: UniformListScrollHandle::new(),
            wants_plan: false,
            envs_by_tag: BTreeMap::new(),
            window: None,
            _subs: subs,
        };
        this.follow_cluster(cx);
        this
    }

    /// The env chips: every env's deployments of this repository's apps (an open panel
    /// wants every env, like the matrix).
    fn follow_cluster(&mut self, cx: &mut Context<Self>) {
        let services = self.ctx.services.read(cx);
        let Some(cluster) = services.cluster.clone() else { return };
        let apps: Vec<_> = services.catalog.apps.iter().filter(|a| crate::repo::root_of(&a.dir, None).as_deref() == Some(self.root.as_path())).cloned().collect();
        let all_apps = services.catalog.apps.clone();
        cluster.update(cx, |c, cx| c.want_all(cx));
        self.envs_by_tag = envs_by_tag(cluster.read(cx), &apps, &all_apps);
        cx.notify();
    }

    fn repo_changed(&mut self, cx: &mut Context<Self>) {
        if self.wants_plan
            && let Some(plan) = self.repo.update(cx, |r, _| r.take_plan())
        {
            self.wants_plan = false;
            self.offer_push(plan, cx);
        }
        cx.notify();
    }

    /// Branch rows: local ones, then remote ones without a local branch of their name,
    /// filtered.
    pub fn rows(&self, cx: &App) -> Vec<Row> {
        let Some(s) = self.repo.read(cx).snapshot() else { return Vec::new() };
        let filter = self.filter.read(cx).text().trim().to_lowercase();
        let locals: Vec<&str> = s.branches.iter().filter(|b| b.remote.is_none()).map(|b| b.name.as_str()).collect();
        s.branches
            .iter()
            .filter_map(|b| {
                let local = match &b.remote {
                    None => b.name.clone(),
                    Some(remote) => {
                        let name = b.name.strip_prefix(&format!("{remote}/")).unwrap_or(&b.name).to_string();
                        if locals.contains(&name.as_str()) {
                            return None;
                        }
                        name
                    }
                };
                Some(Row { branch: b.clone(), local })
            })
            .filter(|r| filter.is_empty() || r.branch.name.to_lowercase().contains(&filter))
            .collect()
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.rows(cx).len();
        if n == 0 {
            return;
        }
        let next = match self.selected {
            None => 0,
            Some(i) => (i as isize + delta).clamp(0, n as isize - 1) as usize,
        };
        self.selected = Some(next);
        self.scroll.scroll_to_item(next, gpui::ScrollStrategy::Center);
        cx.notify();
    }

    /// Why `op` can't run now, if it can't (R5.5).
    pub fn refusal(&self, op: &Op, cx: &App) -> Option<String> {
        let repo = self.repo.read(cx);
        if let Some(why) = repo.refusal(op, cx) {
            return Some(why);
        }
        let s = repo.full()?;
        let on_branch = matches!(s.status.head, Head::Branch(_));
        match op {
            Op::Pull(_) => match &s.status.upstream {
                _ if !on_branch => Some("not on a branch".into()),
                None => Some("the branch has no upstream to pull from".into()),
                Some(u) if u.gone => Some(format!("{} is gone from the remote", u.name)),
                Some(u) if u.behind == 0 => Some("already up to date (fetch first to see new commits)".into()),
                Some(u) if u.ahead > 0 => Some(format!("your branch has {} commits of its own: pull would need a merge or a rebase, do it in a terminal", u.ahead)),
                Some(_) => None,
            },
            Op::Push(_) if !on_branch => Some("not on a branch".into()),
            Op::Push(_) => match &s.status.head {
                Head::Branch(b) => devdeck_core::git::never_pushed(b).err(),
                _ => None,
            },
            Op::Switch(b) | Op::Track(b) => {
                let row = s.branches.iter().find(|x| &x.name == b);
                match row {
                    Some(x) if x.current => Some("already on it".into()),
                    Some(Branch { worktree: Some(w), .. }) => Some(format!("checked out in another worktree ({})", devdeck_core::config::shown(w))),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn toast(&self, text: String, cx: &mut App) {
        toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
    }

    pub fn fetch(&mut self, cx: &mut Context<Self>) {
        if let Err(why) = self.repo.update(cx, |r, cx| r.run(Op::Fetch, "Fetch".into(), "fetching…".into(), cx)) {
            self.toast(format!("Fetch: {why}"), cx);
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.repo.update(cx, |r, cx| r.refresh(cx));
    }

    pub fn pull(&mut self, cx: &mut Context<Self>) {
        let Some(s) = self.repo.read(cx).full().cloned() else { return };
        let (Head::Branch(branch), Some(up)) = (&s.status.head, &s.status.upstream) else {
            return self.toast("Pull: not on a branch with an upstream".into(), cx);
        };
        let op = Op::Pull(branch.clone());
        if let Some(why) = self.refusal(&op, cx) {
            return self.toast(format!("Pull: {why}"), cx);
        }
        let lines = vec![
            format!("Brings {branch} up to date with {}: {} new commits.", up.name, up.behind),
            "Only a fast-forward: refused if it would need a merge, and nothing is changed then.".into(),
            "Refused too if you switched branch meanwhile.".into(),
        ];
        self.confirm(op, format!("Pull {branch}"), format!("pulling {branch}…"), "Pull", lines, cx);
    }

    /// Asks what a push would do (the remote is fetched first), then offers it.
    pub fn push(&mut self, cx: &mut Context<Self>) {
        if let Some(why) = self.refusal(&Op::Push(PushPlan::UpToDate { remote: String::new(), branch: String::new() }), cx) {
            return self.toast(format!("Push: {why}"), cx);
        }
        match self.repo.update(cx, |r, cx| r.plan_push(cx)) {
            Ok(()) => self.wants_plan = true,
            Err(why) => self.toast(format!("Push: {why}"), cx),
        }
    }

    fn offer_push(&mut self, plan: Result<PushPlan, String>, cx: &mut Context<Self>) {
        let plan = match plan {
            Ok(p) => p,
            Err(why) => return self.toast(format!("Push: {why}"), cx),
        };
        let (title, lines, label) = match &plan {
            PushPlan::Publish { remote, branch } => {
                (format!("Publish {branch}"), vec![format!("{branch} isn't on {remote} yet: it's published there, and later pushes go there.")], "Publish")
            }
            PushPlan::Send { remote, branch, commits, .. } => {
                (format!("Push {branch}"), vec![format!("Sends {commits} commits of {branch} to {remote}."), "Refused if your branch moved since this check.".into()], "Push")
            }
            PushPlan::Replace { remote, branch, .. } if crate::edit_mode::refusal(cx).is_some() => {
                return self.toast(format!("Push: your {branch} was rewritten, so pushing it replaces {remote}'s: {}.", crate::edit_mode::OFF), cx);
            }
            PushPlan::Replace { remote, branch, replaced, .. } => {
                let mut lines = vec![
                    format!("Your {branch} was rewritten (a rebase or an amend): {remote}'s {branch} is replaced with yours."),
                    format!("These {} commits on {remote} are replaced; each is yours, or its change is already in your branch:", replaced.len()),
                ];
                lines.extend(replaced.iter().take(10).map(|c| format!("  {} {} ({})", &c.id[..c.id.len().min(8)], c.subject, c.author)));
                if replaced.len() > 10 {
                    lines.push(format!("  and {} more", replaced.len() - 10));
                }
                lines.push(format!("A copy of {remote}'s {branch} is kept in your repository (refs/devdeck/replaced/…), so nothing is lost."));
                lines.push("If someone pushed to it since the check, or your branch moved, the push is refused and nothing changes.".into());
                (format!("Replace {remote}'s {branch}"), lines, "Replace")
            }
            PushPlan::Behind { remote, branch, commits } => {
                return self.toast(format!("Push refused: {remote}'s {branch} has {commits} commits you don't have. Pull them first, or handle it in a terminal."), cx);
            }
            PushPlan::UpToDate { remote, branch } => {
                toasts(cx).update(cx, |t, cx| t.push(ToastKind::Info, format!("{branch} is up to date on {remote}."), vec![], cx));
                return;
            }
        };
        let doing = format!("{}…", title.to_lowercase());
        self.confirm(Op::Push(plan), title, doing, label, lines, cx);
    }

    /// The selected row's switch (a remote branch: a local branch tracking it).
    pub fn switch_selected(&mut self, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        let Some(row) = self.selected.and_then(|i| rows.get(i)).cloned() else { return };
        self.switch(&row, cx);
    }

    pub fn switch(&mut self, row: &Row, cx: &mut Context<Self>) {
        let op = match &row.branch.remote {
            None => Op::Switch(row.branch.name.clone()),
            Some(_) => Op::Track(row.branch.name.clone()),
        };
        if let Some(why) = self.refusal(&op, cx) {
            return self.toast(format!("Switch to {}: {why}", row.local), cx);
        }
        // The changes as they are now: edits in the working tree alone don't refresh it.
        let Some(git) = self.repo.read(cx).git() else { return };
        let row = row.clone();
        cx.spawn(async move |this, cx| {
            let status = cx.background_executor().spawn(async move { git.status() }).await;
            let _ = this.update(cx, |v, cx| match status {
                Ok(s) => {
                    let changes = s.changes.iter().filter(|c| c.kind != ChangeKind::Untracked).count();
                    v.ask_switch(&row, op, changes, cx);
                }
                Err(e) => v.toast(format!("Switch to {}: {e}", row.local), cx),
            });
        })
        .detach();
    }

    fn ask_switch(&mut self, row: &Row, op: Op, changes: usize, cx: &mut Context<Self>) {
        let mut lines = vec![match &row.branch.remote {
            None => format!("Switches to {}.", row.local),
            Some(remote) => format!("Creates {} from {remote}'s and switches to it; it follows {} from then on.", row.local, row.branch.name),
        }];
        if changes > 0 {
            lines.push(format!("Your {changes} uncommitted changes come along to {}. If any would be overwritten, git refuses and nothing changes.", row.local));
        }
        lines.push("Ignored files (a local .env) are never overwritten: git refuses instead.".into());
        let local = row.local.clone();
        self.confirm(op, format!("Switch to {local}"), format!("switching to {local}…"), "Switch", lines, cx);
    }

    pub fn new_branch(&mut self, cx: &mut Context<Self>) {
        if let Some(why) = self.refusal(&Op::Create(String::new()), cx) {
            return self.toast(format!("New branch: {why}"), cx);
        }
        let from = match self.repo.read(cx).full().map(|s| s.status.head.clone()) {
            Some(Head::Branch(b) | Head::Unborn(b)) => b,
            Some(Head::Detached(c)) => c,
            None => return,
        };
        let me = cx.entity();
        self.in_window(cx, move |window, cx| {
            let modal = cx.new(|cx| NewBranchModal::new(me, from, window, cx));
            show(modal, window, cx);
        });
    }

    /// Asks to abort the operation under way, saying which files' changes it resets.
    pub fn abort(&mut self, doing: Doing, cx: &mut Context<Self>) {
        if let Some(why) = self.refusal(&Op::Abort(doing), cx) {
            return self.toast(format!("Abort: {why}"), cx);
        }
        let Some(git) = self.repo.read(cx).git() else { return };
        cx.spawn(async move |this, cx| {
            let impact = cx.background_executor().spawn(async move { git.abort_impact() }).await;
            let _ = this.update(cx, |v, cx| match impact {
                Ok(files) => v.ask_abort(doing, files, cx),
                Err(e) => v.toast(format!("Abort: {e}"), cx),
            });
        })
        .detach();
    }

    fn ask_abort(&mut self, doing: Doing, files: Vec<String>, cx: &mut Context<Self>) {
        let (what, mut lines) = match doing {
            Doing::Rebasing => ("the rebase", vec!["The branch goes back to where it was before the rebase started.".to_string(), "Conflict resolutions made so far are lost.".into()]),
            Doing::Merging => ("the merge", vec!["The branch goes back to where it was before the merge.".to_string(), "Conflict resolutions made so far are lost.".into()]),
            Doing::CherryPicking => ("the cherry-pick", vec!["The branch goes back to where it was before the cherry-pick.".to_string(), "Conflict resolutions made so far are lost.".into()]),
            Doing::Reverting => ("the revert", vec!["The branch goes back to where it was before the revert.".to_string(), "Conflict resolutions made so far are lost.".into()]),
            Doing::Bisecting => ("the bisect", vec!["Back to the commit checked out before the bisect started.".to_string()]),
            Doing::Applying => ("the patch series (git am)", vec!["The branch goes back to where it was before the patches.".to_string(), "Conflict resolutions made so far are lost.".into()]),
        };
        if !files.is_empty() {
            lines.push(format!("The changes in these {} files are reset too:", files.len()));
            lines.extend(files.iter().take(12).map(|f| format!("  {f}")));
            if files.len() > 12 {
                lines.push(format!("  and {} more", files.len() - 12));
            }
        }
        self.confirm(Op::Abort(doing), format!("Abort {what}"), format!("aborting {what}…"), "Abort", lines, cx);
    }

    /// Asks in a modal, then runs `op`.
    fn confirm(&mut self, op: Op, title: String, doing: String, confirm: &'static str, lines: Vec<String>, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        self.in_window(cx, move |window, cx| {
            let modal = cx.new(|cx| ConfirmModal::new(repo, op, title, doing, confirm, lines, window, cx));
            show(modal, window, cx);
        });
    }

    fn in_window(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut Window, &mut App) + 'static) {
        let Some(handle) = self.window else { return };
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| f(window, cx));
        });
    }

    /// The envs running `row`'s branch.
    fn envs_of(&self, row: &Row) -> Vec<(String, bool)> {
        let exact = self.envs_by_tag.get(&row.local);
        let sanitized = self.envs_by_tag.get(&tag_of(&row.local));
        exact.or(sanitized).cloned().unwrap_or_default()
    }
}

/// A branch name as a Docker tag: what a tag can't hold (`[A-Za-z0-9_.-]` only) as `-`.
pub fn tag_of(branch: &str) -> String {
    branch.chars().map(|c| if c.is_ascii_alphanumeric() || "_.-".contains(c) { c } else { '-' }).collect()
}

/// The tags each env runs for `apps` (this repository's), by tag: env and protected.
fn envs_by_tag(c: &Cluster, apps: &[devdeck_core::apps::App], all: &[devdeck_core::apps::App]) -> BTreeMap<String, Vec<(String, bool)>> {
    let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
    let mut out: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    // A deployment is on several envs: its app is looked up once.
    let mut linked: std::collections::HashMap<(String, String), Option<String>> = std::collections::HashMap::new();
    for env in &c.envs {
        let Some(deps) = c.state(&env.name).and_then(|s| s.deployments.as_ref()) else { continue };
        for d in deps.values() {
            let image = d.main().image.short_name().to_string();
            let app = linked.entry((d.name.clone(), image)).or_insert_with_key(|(name, image)| devdeck_core::links::app_for(name, image, all).ok().flatten());
            let Some(app) = app else { continue };
            if ids.contains(&app.as_str()) {
                let envs = out.entry(d.tag()).or_default();
                if !envs.iter().any(|(e, _)| *e == env.name) {
                    envs.push((env.name.clone(), env.protected));
                }
            }
        }
    }
    out
}

fn show<M: Render + HasFocus>(modal: Entity<M>, window: &mut Window, cx: &mut App) {
    push_modal(modal.clone().into(), window, cx);
    let first = modal.read(cx).focus().0[0].clone();
    first.focus(window, cx);
}

trait HasFocus {
    fn focus(&self) -> &ModalFocus;
}

impl Focusable for GitView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn age(now: u64, at: u64) -> String {
    super::fmt_age(now.saturating_sub(at))
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Render for GitView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.window = Some(window.window_handle());
        let t = theme(cx).clone();
        let root = div()
            .id("git-view")
            .key_context("GitView")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &SwitchSelected, _, cx| this.switch_selected(cx)))
            .on_action(cx.listener(|this, _: &Fetch, _, cx| this.fetch(cx)))
            .on_action(cx.listener(|this, _: &Pull, _, cx| this.pull(cx)))
            .on_action(cx.listener(|this, _: &Push, _, cx| this.push(cx)))
            .on_action(cx.listener(|this, _: &NewBranch, _, cx| this.new_branch(cx)))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)));
        let repo = self.repo.read(cx);
        let message = |text: String, color| div().flex_1().flex().items_center().justify_center().text_color(color).child(label(text));
        let snapshot = match (&repo.state, repo.full()) {
            (Remote::Error(e), _) => return root.child(div().flex_1().flex().items_center().justify_center().child(icon_text(Icon::Warning, t.warn, e.clone(), &t))),
            (_, Some(s)) => s.clone(),
            (_, None) => return root.child(message(format!("reading {}…", devdeck_core::config::shown(&self.root)), t.mute)),
        };
        let busy = repo.busy_text();
        let last = repo.last.clone();
        let now = now_s();
        let status = &snapshot.status;
        let head = match &status.head {
            Head::Branch(b) => b.clone(),
            Head::Detached(c) => format!("detached at {c}"),
            Head::Unborn(b) => format!("{b} (no commit yet)"),
        };
        let upstream = match &status.upstream {
            None if matches!(status.head, Head::Branch(_)) => "no upstream".to_string(),
            None => String::new(),
            Some(u) if u.gone => format!("{} is gone from the remote", u.name),
            Some(u) => match (u.ahead, u.behind) {
                (0, 0) => format!("up to date with {}", u.name),
                (a, 0) => format!("{a} ahead of {}", u.name),
                (0, b) => format!("{b} behind {}", u.name),
                (a, b) => format!("{a} ahead, {b} behind {}", u.name),
            },
        };
        let changed = status.changes.len();
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
                    .child(div().flex_none().text_color(t.hi).child(head))
                    .child(label(upstream).text_color(t.mute).flex_none())
                    .child(label(if changed == 0 { "clean".to_string() } else { format!("{changed} changes") }).text_color(if changed == 0 { t.dim } else { t.warn }).flex_none())
                    .child(label(devdeck_core::config::shown(&self.root)).text_color(t.dim).min_w_0()),
            )
            .when_some(snapshot.blocked.clone(), |d, why| {
                let doing = status.doing;
                d.child(
                    div().flex().items_center().gap_2().child(icon_text(Icon::Warning, t.warn, why, &t).flex_1()).when_some(doing, |d, doing| {
                        let why = self.refusal(&Op::Abort(doing), cx);
                        d.child(
                            button("git-abort", "abort", &t)
                                .debug_selector(|| "git-abort".into())
                                .when(why.is_some(), |b| b.text_color(t.dim))
                                .tooltip(tip(why.unwrap_or_else(|| "abort it, back to where it started".into())))
                                .on_click(cx.listener(move |this, _, _, cx| this.abort(doing, cx))),
                        )
                    }),
                )
            })
            .when_some(busy.clone(), |d, b| d.child(icon_text(Icon::Busy, t.accent, b, &t)))
            .when_some(last, |d, o| {
                let (text, color, full) = match &o.result {
                    Ok(out) => (format!("{}: done", o.what), t.dim, out.clone()),
                    Err(e) => (format!("{}: {e}", o.what), t.warn, e.clone()),
                };
                d.child(div().id("git-last").child(label(text).text_color(color)).tooltip(tip(if full.is_empty() { "nothing to say".to_string() } else { full })))
            });

        let action = |id: &'static str, text: &'static str, why: Option<String>| {
            let enabled = why.is_none();
            button(id, text, &t).when(!enabled, |b| b.text_color(t.dim)).tooltip(tip(why.unwrap_or_else(|| text.to_string())))
        };
        let push_probe = Op::Push(PushPlan::UpToDate { remote: String::new(), branch: String::new() });
        let pull_branch = match &status.head {
            Head::Branch(b) => b.clone(),
            _ => String::new(),
        };
        let actions = div()
            .flex_none()
            .px_3()
            .py_1()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(t.line_soft)
            .child(action("git-fetch", "fetch", self.refusal(&Op::Fetch, cx)).on_click(cx.listener(|this, _, _, cx| this.fetch(cx))))
            .child(action("git-pull", "pull", self.refusal(&Op::Pull(pull_branch), cx)).on_click(cx.listener(|this, _, _, cx| this.pull(cx))))
            .child(action("git-push", "push", self.refusal(&push_probe, cx)).on_click(cx.listener(|this, _, _, cx| this.push(cx))))
            .child(action("git-new", "new branch…", self.refusal(&Op::Create(String::new()), cx)).on_click(cx.listener(|this, _, _, cx| this.new_branch(cx))))
            .child(action("git-refresh", "refresh", None).on_click(cx.listener(|this, _, _, cx| this.refresh(cx))));

        let mut changes = div().flex().flex_col();
        for c in status.changes.iter().take(CHANGES_SHOWN) {
            let kind = match c.kind {
                ChangeKind::Modified => "modified",
                ChangeKind::Added => "added",
                ChangeKind::Deleted => "deleted",
                ChangeKind::Renamed => "renamed",
                ChangeKind::Copied => "copied",
                ChangeKind::TypeChanged => "type changed",
                ChangeKind::Untracked => "untracked",
                ChangeKind::Conflicted => "conflict",
            };
            let path = match &c.from {
                Some(from) => format!("{from} → {}", c.path),
                None => c.path.clone(),
            };
            let color = if c.kind == ChangeKind::Conflicted { t.err } else { t.mute };
            changes = changes.child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_none().w(t.size(110.)).text_color(color).child(format!("{kind}{}", if c.staged { " · staged" } else { "" })))
                    .child(label(path).text_color(t.fg).min_w_0()),
            );
        }
        if status.changes.len() > CHANGES_SHOWN {
            changes = changes.child(label(format!("and {} more", status.changes.len() - CHANGES_SHOWN)).text_color(t.dim));
        }

        let rows = self.rows(cx);
        let count = rows.len();
        let list = uniform_list(
            "git-branches",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = theme(cx).clone();
                let rows = this.rows(cx);
                range.filter_map(|i| rows.get(i).map(|r| this.row(i, r, now, &t, cx))).collect()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();

        root.child(header).child(actions).child(
            div()
                .flex_1()
                .min_h_0()
                .px_3()
                .pb_2()
                .flex()
                .flex_col()
                .gap_1()
                .when(!status.changes.is_empty(), |d| d.child(section("Changes", &t)).child(changes))
                .child(section("Branches", &t))
                .child(div().flex_none().debug_selector(|| "git-filter".into()).child(self.filter.clone()))
                .child(if count == 0 { label("no branches match").text_color(t.dim).into_any_element() } else { list.into_any_element() }),
        )
    }
}

impl GitView {
    fn row(&self, i: usize, r: &Row, now: u64, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let b = &r.branch;
        let selected = self.selected == Some(i);
        let envs = self.envs_of(r);
        let row = r.clone();
        let menu_row = r.clone();
        let hover = t.hover;
        div()
            .id(("branch", i))
            .debug_selector({
                let id = format!("branch-{}", b.name);
                move || id.clone()
            })
            .h(t.row_h)
            .flex()
            .items_center()
            .gap_2()
            .px_1()
            .rounded(t.radius_s)
            .when(selected, |d| d.bg(t.selection))
            .hover(move |s| s.bg(hover))
            .cursor_pointer()
            .child(div().flex_none().w(t.icon).child(if b.current { icon(Icon::Check, t.ok, t).into_any_element() } else { div().into_any_element() }))
            .child(label(b.name.clone()).flex_none().max_w(t.size(320.)).text_color(if b.remote.is_some() { t.mute } else { t.fg }))
            .children(envs.into_iter().map(|(e, protected)| {
                let color = if protected { t.err } else { t.env };
                div().flex_none().px_1().rounded(t.radius_s).text_color(color).child(format!("@{e}"))
            }))
            .child(label(b.subject.clone()).text_color(t.mute).flex_1().min_w_0())
            .child(div().flex_none().text_color(t.dim).child(format!("{} · {}", b.author, age(now, b.at))))
            .when_some(b.worktree.clone(), |d, w| d.child(div().flex_none().text_color(t.dim).child(format!("in {}", devdeck_core::config::shown(&w)))))
            .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                this.selected = Some(i);
                if ev.click_count() == 2 {
                    this.switch(&row, cx);
                }
                cx.notify();
            }))
            .on_mouse_down(gpui::MouseButton::Right, {
                let me = cx.entity();
                move |e, window, cx| {
                    let (me, row) = (me.clone(), menu_row.clone());
                    let label = if row.branch.remote.is_some() { format!("Switch to {} (from {})", row.local, row.branch.name) } else { format!("Switch to {}", row.local) };
                    let why = {
                        let op = if row.branch.remote.is_some() { Op::Track(row.branch.name.clone()) } else { Op::Switch(row.branch.name.clone()) };
                        me.read(cx).refusal(&op, cx)
                    };
                    let item = devdeck_ui::MenuItem::new(label, move |_, cx| me.update(cx, |v, cx| v.switch(&row, cx)));
                    devdeck_ui::open_menu(e.position, vec![match why {
                        Some(why) => item.disabled(why),
                        None => item,
                    }], window, cx);
                }
            })
            .into_any_element()
    }
}

// ------------------------------------------------------------------ modals

/// Says in plain words what an operation will do; Enter runs it, Esc cancels.
pub struct ConfirmModal {
    repo: Entity<Repo>,
    op: Op,
    title: String,
    doing: String,
    confirm_label: &'static str,
    lines: Vec<String>,
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
}

impl ConfirmModal {
    #[allow(clippy::too_many_arguments)]
    fn new(repo: Entity<Repo>, op: Op, title: String, doing: String, confirm_label: &'static str, lines: Vec<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let restore = window.focused(cx).map(|f| f.downgrade());
        Self { repo, op, title, doing, confirm_label, lines, focus: ModalFocus(vec![cx.focus_handle(), cx.focus_handle()]), restore }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn go(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
        let (op, title, doing) = (self.op.clone(), self.title.clone(), self.doing.clone());
        if let Err(why) = self.repo.update(cx, |r, cx| r.run(op, title.clone(), doing, cx)) {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("{title}: {why}"), vec![], cx));
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.0[1].is_focused(window) {
            return self.close(window, cx);
        }
        self.go(window, cx);
    }
}

impl HasFocus for ConfirmModal {
    fn focus(&self) -> &ModalFocus {
        &self.focus
    }
}

fn focus_button(id: &'static str, text: &'static str, handle: &FocusHandle, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let ring = t.focus;
    button(id, text, t).border_1().border_color(t.line).track_focus(handle).focus(move |s| s.border_color(ring))
}

impl Render for ConfirmModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let content = div()
            .key_context("Modal GitModal")
            .flex()
            .flex_col()
            .gap_2()
            .max_w(t.size(720.))
            .on_action(cx.listener(|this, _: &devdeck_ui::overlay::ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &devdeck_ui::overlay::ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &devdeck_ui::overlay::ModalClose, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &ConfirmGit, window, cx| this.confirm(window, cx)))
            .child(div().text_color(t.hi).debug_selector(|| "git-confirm-title".into()).child(format!("{}?", self.title)))
            // Text a modal shows whole (R4.4 allows a view to opt in): what happens matters.
            .children(self.lines.iter().map(|l| div().text_color(t.fg).child(l.clone())))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(focus_button("git-cancel", "Cancel", &self.focus.0[1], &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))))
                    .child(focus_button("git-confirm", self.confirm_label, &self.focus.0[0], &t).debug_selector(|| "git-confirm".into()).on_click(cx.listener(|this, _, w, cx| this.go(w, cx)))),
            );
        modal_frame(content.into_any_element(), window, &t)
    }
}

/// A new branch from the current one: its name, Enter creates it and switches to it.
pub struct NewBranchModal {
    view: Entity<GitView>,
    from: String,
    input: Entity<TextInput>,
    error: Option<SharedString>,
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
    _sub: Subscription,
}

impl NewBranchModal {
    fn new(view: Entity<GitView>, from: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let restore = window.focused(cx).map(|f| f.downgrade());
        let input = cx.new(|cx| TextInput::new("branch name", cx));
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| {
            if let InputEvent::Submit(_) = e {
                this.submit(window, cx);
            }
        });
        let handles = vec![input.read(cx).focus_handle(cx), cx.focus_handle()];
        Self { view, from, input, error: None, focus: ModalFocus(handles), restore, _sub: sub }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.input.read(cx).text().trim().to_string();
        if name.is_empty() {
            self.error = Some("type a name".into());
            return cx.notify();
        }
        let repo = self.view.read(cx).repo.clone();
        let what = format!("New branch {name}");
        let result = repo.update(cx, |r, cx| r.run(Op::Create(name.clone()), what, format!("creating {name}…"), cx));
        match result {
            Ok(()) => self.close(window, cx),
            Err(why) => {
                self.error = Some(why.into());
                cx.notify();
            }
        }
    }
}

impl HasFocus for NewBranchModal {
    fn focus(&self) -> &ModalFocus {
        &self.focus
    }
}

impl Render for NewBranchModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let content = div()
            .key_context("Modal GitModal")
            .flex()
            .flex_col()
            .gap_2()
            .w(t.size(520.))
            .on_action(cx.listener(|this, _: &devdeck_ui::overlay::ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &devdeck_ui::overlay::ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &devdeck_ui::overlay::ModalClose, window, cx| this.close(window, cx)))
            .child(div().text_color(t.hi).child(format!("New branch from {}", self.from)))
            .child(label("It's created from the current commit and switched to; your uncommitted changes come along.").text_color(t.mute))
            .child(div().debug_selector(|| "git-new-name".into()).child(self.input.clone()))
            .when_some(self.error.clone(), |d, e| d.child(icon_text(Icon::Warning, t.warn, e.to_string(), &t)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(focus_button("git-new-cancel", "Cancel", &self.focus.0[1], &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))))
                    .child(button("git-new-create", "Create", &t).on_click(cx.listener(|this, _, w, cx| this.submit(w, cx)))),
            );
        modal_frame(content.into_any_element(), window, &t)
    }
}
