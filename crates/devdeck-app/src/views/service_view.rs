//! Service inspector (`svc/<env>/<name>`, R7.2–R7.6, R7.8): header, actions (redeploy the
//! same branch among them), the tag row (the tag editor with its suggestions, the deploy
//! rocket or ctrl+s → confirmation stating the exact command, waiting for the cluster's
//! answer; the notify dropdown; the unsaved marker), the deploy followed to its end,
//! protected-env unlock, then metrics, pods, container and the deploy history. Opening it
//! asks the cluster whether the account may change deployments there.
//!
//! Metrics are sampled only while the view is visible (it holds a [`Sampling`] handle).
//! Keys (`ServiceView`): ctrl+s deploy, l logs, t edit tag, r redeploy, b notify; Esc in the
//! tag editor reverts it (the pane then gives the keys back to the view). What the notify
//! dropdown ticks is the cluster's ([`crate::cluster_notes`]): it outlives the view.

use super::{ViewCx, fmt_age};
use crate::cluster::{Cluster, PodScope, PodsInterest, Sampling, WriteState};
use crate::repo::{Looking, Repo};
use devdeck_core::cluster::{Deployment, Pod, Remote, RolloutState, redeploy_warning, unlock_matches};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::Scrolls;
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{
    Icon, InputEvent, MenuItem, ModalFocus, TextInput, Theme, ToastKind, button, dot, icon, icon_action_ready, icon_button, icon_text, label, modal_frame,
    notify_button, open_menu, pop_modal, push_modal, section, theme, tip, toasts, under,
};
use std::cell::Cell;
use std::rc::Rc;
use gpui::{
    AnyWindowHandle, App, AppContext, Bounds, Pixels, Context, Entity, FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, WeakFocusHandle, Window, actions, div, prelude::*, relative,
};

actions!(service_view, [SaveTag, OpenLogs, EditTag, Redeploy, NotifyMenu, ConfirmModal]);

pub struct ServiceView {
    pub env: String,
    pub name: String,
    ctx: ViewCx,
    cluster: Entity<Cluster>,
    focus: FocusHandle,
    pub tag: Entity<TextInput>,
    /// The tag shown when the editor isn't dirty.
    shown_tag: Option<String>,
    /// Writes allowed on a protected env until the view closes (R7.6).
    pub unlocked: bool,
    sampling: Option<Sampling>,
    /// Its pods are watched while it's open.
    _pods: PodsInterest,
    /// The app it's deployed from and its repository, once linked (M3 step 3).
    repo: Option<(String, Entity<Repo>)>,
    /// Why it isn't linked to an app, when it isn't.
    unlinked: Option<String>,
    visible: bool,
    looking: Option<Looking>,
    /// The window it last drew in: modals open there.
    window: Option<AnyWindowHandle>,
    /// Where the notify dropdown's button was drawn: `b` opens its menu under it.
    notify_at: Rc<Cell<Option<Bounds<Pixels>>>>,
    _subs: Vec<Subscription>,
}

impl ServiceView {
    pub fn new(env: String, name: String, ctx: ViewCx, cluster: Entity<Cluster>, cx: &mut Context<Self>) -> Self {
        let tag = cx.new(|cx| TextInput::new("image tag", cx));
        // Its buttons say whether edit mode allows them.
        let edit_mode = crate::edit_mode::edit_mode(cx);
        let mut subs = vec![
            cx.observe(&edit_mode, |_, _, cx| cx.notify()),
            cx.observe(&cluster, |this, _, cx| this.cluster_changed(cx)),
            cx.subscribe(&tag, |this, _, e: &InputEvent, cx| match e {
                InputEvent::Submit(_) => this.save(cx),
                InputEvent::Cancel => this.revert(cx),
                InputEvent::Changed(_) => cx.notify(),
            }),
        ];
        subs.extend(ctx.on_visibility(cx, |this, visible, cx| this.set_visible(visible, cx)));
        let pods = cluster.update(cx, |c, cx| {
            c.check_access(&env, cx);
            c.pods_interest(&env, PodScope::Service(name.clone()), cx)
        });
        let mut this = Self {
            env,
            name,
            ctx,
            cluster,
            focus: cx.focus_handle(),
            tag,
            shown_tag: None,
            unlocked: false,
            sampling: None,
            _pods: pods,
            repo: None,
            unlinked: None,
            visible: false,
            looking: None,
            window: None,
            notify_at: Rc::default(),
            _subs: subs,
        };
        this.cluster_changed(cx);
        this
    }

    pub fn is_sampling(&self) -> bool {
        self.sampling.is_some()
    }

    /// Holds a sampling handle only while on screen.
    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let env = self.env.clone();
        self.visible = visible;
        self.sampling = visible.then(|| self.cluster.update(cx, |c, cx| c.sample(&env, cx)));
        self.looking = visible.then(|| self.repo.as_ref().map(|(_, r)| r.update(cx, |r, cx| r.glance(cx)))).flatten();
    }

    /// Links it to the app it's deployed from (by name, image, or the app's
    /// `deployments`), and that app's repository.
    fn link(&mut self, cx: &mut Context<Self>) {
        if self.repo.is_some() {
            return;
        }
        let Some(d) = self.deployment(cx) else { return };
        let image = d.main().image.short_name().to_string();
        let found = devdeck_core::links::app_for(&d.name, &image, &self.ctx.services.read(cx).catalog.apps);
        let id = match found {
            Ok(Some(id)) => id,
            Ok(None) => {
                self.unlinked = Some(format!("no app of this workspace deploys as {}: add `deployments = [\"{}\"]` to its app in the workspace file", d.name, d.name));
                return;
            }
            Err(problem) => {
                self.unlinked = Some(problem);
                return;
            }
        };
        let Some(dir) = self.ctx.services.read(cx).catalog.app(&id).map(|a| a.dir.clone()) else { return };
        let Some(repo) = self.ctx.services.update(cx, |s, cx| s.repo(&dir, cx)) else {
            self.unlinked = Some(format!("{id} isn't in a git repository"));
            return;
        };
        self.unlinked = None;
        self._subs.push(cx.observe(&repo, |_, _, cx| cx.notify()));
        if self.visible {
            self.looking = Some(repo.update(cx, |r, cx| r.glance(cx)));
        }
        self.repo = Some((id, repo));
    }

    fn open_git(&mut self, cx: &mut Context<Self>) {
        let Some((_, repo)) = &self.repo else { return };
        let root = repo.read(cx).root.display().to_string();
        self.ctx.open(ViewAddr::Git { root }, cx);
    }

    /// Keeps the editor on the deployed tag unless the user changed it.
    fn cluster_changed(&mut self, cx: &mut Context<Self>) {
        self.link(cx);
        let current = self.deployment(cx).map(|d| d.tag());
        if current != self.shown_tag {
            let dirty = self.dirty(cx);
            self.shown_tag = current.clone();
            if !dirty && let Some(t) = current {
                self.tag.update(cx, |i, cx| i.set_text(&t, cx));
            }
        }
        cx.notify();
    }

    fn deployment(&self, cx: &App) -> Option<Deployment> {
        self.cluster.read(cx).deployment(&self.env, &self.name).cloned()
    }

    /// The editor holds something other than the deployed tag.
    pub fn dirty(&self, cx: &App) -> bool {
        let text = self.tag.read(cx).text().trim().to_string();
        self.shown_tag.as_ref().is_some_and(|t| *t != text) && !text.is_empty()
    }

    fn protected(&self, cx: &App) -> bool {
        self.cluster.read(cx).env(&self.env).is_some_and(|e| e.protected)
    }

    /// Why the deploy (the rocket, ctrl+s) can't go ahead now, if it can't.
    pub fn save_blocked(&self, cx: &App) -> Option<String> {
        // Edit mode first: it's what the user can do something about.
        if let Err(why) = self.cluster.read(cx).writes(cx) {
            return Some(why);
        }
        if !self.dirty(cx) {
            return Some("the tag is unchanged".into());
        }
        let to = self.tag.read(cx).text().trim().to_string();
        self.cluster.read(cx).deploy_blocked(&self.env, &self.name, &to, self.unlocked, cx)
    }

    /// The rocket or ctrl+s: on a locked protected env asks to unlock first; otherwise
    /// confirms.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        // Edit mode before the protected env's name: never ask the name to then refuse.
        // Everything else before the protected env's name: never ask it to then refuse.
        let to = self.tag.read(cx).text().trim().to_string();
        let other = self.cluster.read(cx).deploy_blocked(&self.env, &self.name, &to, true, cx);
        if other.is_none() && self.protected(cx) && !self.unlocked && self.dirty(cx) {
            self.request_unlock(cx);
            return;
        }
        if let Some(why) = self.save_blocked(cx) {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("Not deployed: {why}."), vec![], cx));
            return;
        }
        let to = self.tag.read(cx).text().trim().to_string();
        let c = self.cluster.read(cx);
        let (Some(d), Some(command)) = (c.deployment(&self.env, &self.name), c.command_for(&self.env, &self.name, &to)) else { return };
        let (from_tag, from_image) = (d.tag(), d.main().image.raw.clone());
        let me = cx.entity();
        let (env, name) = (self.env.clone(), self.name.clone());
        self.in_window(cx, move |window, cx| {
            let kind = WriteKind::Change { from: from_tag, from_image, to };
            let modal = cx.new(|cx| ConfirmDeploy::new(me, env, name, kind, command, window, cx));
            show_modal(modal, window, cx);
        });
    }

    /// Why a redeploy can't go ahead now, if it can't (every guard of a write: edit mode
    /// first).
    pub fn redeploy_blocked(&self, cx: &App) -> Option<String> {
        self.cluster.read(cx).write_blocked(&self.env, &self.name, self.unlocked, cx)
    }

    /// `r`: on a locked protected env asks to unlock first (after every other guard);
    /// otherwise confirms the redeploy of the image running now.
    pub fn redeploy(&mut self, cx: &mut Context<Self>) {
        let other = self.cluster.read(cx).write_blocked(&self.env, &self.name, true, cx);
        if other.is_none() && self.protected(cx) && !self.unlocked {
            self.request_unlock(cx);
            return;
        }
        if let Some(why) = self.redeploy_blocked(cx) {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("Not redeployed: {why}."), vec![], cx));
            return;
        }
        let c = self.cluster.read(cx);
        let (Some(d), Some(command)) = (c.deployment(&self.env, &self.name), c.restart_command_for(&self.env, &self.name)) else { return };
        let Ok((_, main)) = d.deploy_target() else { return };
        let kind = WriteKind::Redeploy { tag: d.tag(), image: main.image.raw.clone(), warning: redeploy_warning(main) };
        let me = cx.entity();
        let (env, name) = (self.env.clone(), self.name.clone());
        self.in_window(cx, move |window, cx| {
            let modal = cx.new(|cx| ConfirmDeploy::new(me, env, name, kind, command, window, cx));
            show_modal(modal, window, cx);
        });
    }

    /// Sends the redeploy confirmed, pinned to the image that was shown.
    pub fn send_redeploy(&mut self, image: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let (env, name, unlocked) = (self.env.clone(), self.name.clone(), self.unlocked);
        let result = self.cluster.update(cx, |c, cx| c.redeploy(&env, &name, image, unlocked, cx));
        cx.notify();
        result
    }

    /// Sends the change confirmed: `to` on the image `from` that was shown.
    pub fn deploy(&mut self, from: &str, to: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let (env, name, unlocked) = (self.env.clone(), self.name.clone(), self.unlocked);
        let result = self.cluster.update(cx, |c, cx| c.deploy(&env, &name, from, to, unlocked, cx));
        cx.notify();
        result
    }

    fn revert(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.shown_tag.clone() {
            self.tag.update(cx, |i, cx| i.set_text(&t, cx));
        }
        cx.notify();
    }

    fn request_unlock(&mut self, cx: &mut Context<Self>) {
        let me = cx.entity();
        let env = self.env.clone();
        self.in_window(cx, move |window, cx| {
            let modal = cx.new(|cx| UnlockEnv::new(me, env, window, cx));
            show_modal(modal, window, cx);
        });
    }

    /// Runs `f` in the view's window once the current update is done.
    fn in_window(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut Window, &mut App) + 'static) {
        let Some(handle) = self.window else { return };
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| f(window, cx));
        });
    }

    /// The notify dropdown's options (snapshots and tests read them).
    pub fn notify_items(&self, cx: &App) -> Vec<MenuItem> {
        crate::notifications::notify_items(&self.cluster, &self.env, &self.name, cx)
    }

    /// Opens the notify dropdown under its button (a click, or `b`).
    pub fn open_notify(&self, window: &mut Window, cx: &mut App) {
        let at = under(self.notify_at.get());
        let items = self.notify_items(cx);
        // Opened from a click or a key handler: after it, so the menu keeps the focus.
        window.defer(cx, move |window, cx| open_menu(at, items, window, cx));
    }

    fn open_logs(&mut self, cx: &mut Context<Self>) {
        self.ctx.open(ViewAddr::Logs { env: self.env.clone(), name: self.name.clone() }, cx);
    }

    /// The tag suggestions menu (snapshots open it directly).
    pub fn suggestions_for(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        Self::suggestions(this, cx)
    }

    fn suggestions(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let s = this.read(cx);
        let mut list = s.cluster.read(cx).tag_suggestions(&s.env, &s.name);
        // The repository's remote branches: a branch only local was never built.
        if let Some(snapshot) = s.repo.as_ref().and_then(|(_, r)| r.read(cx).snapshot()) {
            for b in snapshot.branches.iter().filter(|b| b.remote.is_some()) {
                let remote = b.remote.as_deref().unwrap_or_default();
                let name = b.name.strip_prefix(&format!("{remote}/")).unwrap_or(&b.name).to_string();
                if !list.iter().any(|(t, _)| *t == name) && Some(&name) != s.shown_tag.as_ref() {
                    list.push((name, format!("branch on {remote}")));
                }
            }
        }
        if list.is_empty() {
            return vec![MenuItem::new("no other tags known", |_, _| {}).disabled("deploy history is empty")];
        }
        list.into_iter()
            .map(|(tag, why)| {
                let (me, t) = (this.clone(), tag.clone());
                MenuItem::new(tag, move |window, cx| {
                    me.update(cx, |s, cx| {
                        s.tag.update(cx, |i, cx| i.set_text(&t, cx));
                        s.tag.read(cx).focus_handle(cx).focus(window, cx);
                        cx.notify();
                    })
                })
                .hint(why)
            })
            .collect()
    }
}

/// Pushes a modal and focuses its first control.
fn show_modal<M: Render + HasModalFocus>(modal: Entity<M>, window: &mut Window, cx: &mut App) {
    push_modal(modal.clone().into(), window, cx);
    let first = modal.read(cx).modal_focus().0[0].clone();
    first.focus(window, cx);
}

trait HasModalFocus {
    fn modal_focus(&self) -> &ModalFocus;
}

impl Focusable for ServiceView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}


/// `512Mi`, or "none" when unset.
fn quantity(v: Option<u32>, unit: &str) -> String {
    v.map_or_else(|| "none".into(), |v| format!("{v}{unit}"))
}

/// A usage bar against request (tick) and limit (full width); without a limit the bar
/// spans the request, else what's used.
fn bar(used: u32, request: Option<u32>, limit: Option<u32>, t: &Theme) -> impl IntoElement {
    let full = limit.or(request).unwrap_or(used).max(1) as f32;
    let fill = (used as f32 / full).min(1.0);
    let color: Hsla = if request.is_some_and(|r| used > r) { t.warn } else { t.ok };
    div()
        .relative()
        .w(t.size(220.))
        .h(t.size(8.))
        .rounded(t.radius_s)
        .bg(t.line)
        .child(div().absolute().left_0().top_0().h_full().w(relative(fill)).rounded(t.radius_s).bg(color))
        .when_some(request, |d, r| d.child(div().absolute().top_0().h_full().w(t.hairline).left(relative((r as f32 / full).min(1.0))).bg(t.fg)))
}

fn phase_text(p: &Pod) -> (String, bool) {
    (p.phase.text(), p.phase.ok())
}

impl Render for ServiceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let me = cx.entity();
        let c = self.cluster.read(cx);
        let env = c.env(&self.env).cloned();
        let protected = env.as_ref().is_some_and(|e| e.protected);
        let state = c.peek(&self.env);
        let root = div()
            .id("service-view")
            .key_context("ServiceView")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &SaveTag, _, cx| this.save(cx)))
            .on_action(cx.listener(|this, _: &OpenLogs, _, cx| this.open_logs(cx)))
            .on_action(cx.listener(|this, _: &Redeploy, _, cx| this.redeploy(cx)))
            .on_action(cx.listener(|this, _: &NotifyMenu, window, cx| this.open_notify(window, cx)))
            .on_action(cx.listener(|this, _: &EditTag, window, cx| {
                this.tag.read(cx).focus_handle(cx).focus(window, cx);
            }));
        let message = |text: String, color: Hsla| div().flex_1().flex().items_center().justify_center().text_color(color).child(label(text));
        let d = match (&state, c.deployment(&self.env, &self.name)) {
            (Remote::Loading, _) => return root.child(message(format!("loading {}…", self.env), t.mute)),
            (Remote::Error(e), _) => {
                return root.child(div().flex_1().flex().items_center().justify_center().child(icon_text(Icon::Warning, t.warn, e.to_string(), &t)));
            }
            (Remote::Ready(_), None) => return root.child(message(format!("{} isn't deployed in {}.", self.name, self.env), t.mute)),
            (Remote::Ready(_), Some(d)) => d.clone(),
        };
        let rollout = c.rollout(&self.env, &self.name).cloned();
        let pods_state = c.pods_state(&self.env, &self.name);
        let pods: Vec<Pod> = pods_state.ready().map(|p| p.to_vec()).unwrap_or_default();
        let usage = |p: &Pod| c.usage(&self.env, &p.name);
        let history = c.history(&self.env, &self.name).to_vec();
        let metrics = c.metrics_allowed(&self.env);
        let dirty = self.dirty(cx);
        let blocked = self.save_blocked(cx);
        let redeploy_blocked = self.redeploy_blocked(cx);
        let (status_mark, status, status_color) = match rollout.as_ref().map(|r| &r.state) {
            Some(RolloutState::Rolling { ready, of }) => (icon(Icon::Busy, t.accent, &t).into_any_element(), format!("rolling {ready}/{of}"), t.accent),
            Some(RolloutState::Failing { .. }) => (icon(Icon::Error, t.err, &t).into_any_element(), "rollout failing".to_string(), t.err),
            Some(RolloutState::Stuck { .. }) => (icon(Icon::Error, t.err, &t).into_any_element(), "rollout stuck".to_string(), t.err),
            _ if d.ready == d.replicas => (dot(t.ok, true, &t).into_any_element(), format!("ready {}/{}", d.ready, d.replicas), t.ok),
            _ => (icon(Icon::Warning, t.warn, &t).into_any_element(), format!("ready {}/{}", d.ready, d.replicas), t.warn),
        };

        let header = div()
            .flex_none()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .border_b_1()
            .border_color(t.line_soft)
            .child(div().flex_none().text_color(t.hi).child(d.name.clone()))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_color(if protected { t.err } else { t.env })
                    .when(protected, |d| d.child(icon(Icon::Protected, t.err, &t)))
                    .child(format!("@{}", self.env)),
            )
            .when(protected, |h| {
                h.child(div().flex_none().text_color(if self.unlocked { t.warn } else { t.mute }).child(if self.unlocked { "unlocked" } else { "locked" }))
            })
            .child(div().flex_none().flex().items_center().gap_1().text_color(status_color).child(status_mark).child(status))
            .child(
                label(match (d.progress.label(), c.since(&self.env, &d)) {
                    (Some(state), _) => format!("{} · {state}", d.tag()),
                    (None, Some(since)) => format!("{} · since {}", d.tag(), fmt_age(c.now_s().saturating_sub(since))),
                    (None, None) => d.tag(),
                })
                .text_color(t.mute)
                .flex_1(),
            );

        let action = |id: &'static str, text: &'static str, why: Option<String>| {
            let enabled = why.is_none();
            button(id, text, &t).when(!enabled, |b| b.text_color(t.dim)).tooltip(tip(why.unwrap_or_else(|| text.to_string())))
        };
        let later = || Some("comes in a later step of M3".to_string());
        let actions = div()
            .flex_none()
            .px_3()
            .py_1()
            .flex()
            .items_center()
            .gap_1()
            .border_b_1()
            .border_color(t.line_soft)
            .child(action("logs", "logs", None).on_click(cx.listener(|this, _, _, cx| this.open_logs(cx))))
            .child(action("git", "git", self.repo.is_none().then(|| self.unlinked.clone().unwrap_or_else(|| "linking to its app…".into()))).on_click(cx.listener(|this, _, _, cx| this.open_git(cx))))
            .child(action("redeploy", "redeploy", redeploy_blocked).debug_selector(|| "svc-redeploy".into()).on_click(cx.listener(|this, _, _, cx| this.redeploy(cx))))
            .child(action("scale", "replicas", later()))
            .child(action("port-forward", "port-forward", later()))
            .child(action("shell", "shell", later()))
            .when(protected, |a| {
                a.child(div().flex_1()).child(
                    action("lock", if self.unlocked { "lock" } else { "unlock…" }, None).on_click(cx.listener(|this, _, _, cx| {
                        if this.unlocked {
                            this.unlocked = false;
                            cx.notify();
                        } else {
                            this.request_unlock(cx);
                        }
                    })),
                )
            });

        let editor = div()
            .flex_none()
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_none().text_color(t.mute).child("image tag"))
            .child(div().w(t.size(260.)).debug_selector(|| "tag-editor".into()).child(self.tag.clone()))
            .child({
                let me = me.clone();
                icon_button("suggest", Icon::Open, "tags on other envs and history", &t).on_click(move |e, window, cx| {
                    let items = ServiceView::suggestions(&me, cx);
                    open_menu(e.position(), items, window, cx);
                })
            })
            // The rocket: ready (in the accent) when another tag is typed, dim with its reason
            // otherwise (a click says it too).
            .child(
                icon_action_ready("deploy", Icon::Deploy, blocked.clone().unwrap_or_else(|| "deploy".into()), blocked.is_none(), &t)
                    .debug_selector(|| "tag-deploy".into())
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .child({
                let ticked = !c.notify_options(&self.env, &self.name).is_empty();
                notify_button("notify", ticked, self.notify_at.clone(), &t)
                    .debug_selector(|| "tag-notify".into())
                    .on_click(cx.listener(|this, _, window, cx| this.open_notify(window, cx)))
            })
            // Kept in place when clean (hidden), so nothing shifts.
            .child(div().flex_none().flex().items_center().gap_1().text_color(t.warn).when(!dirty, |d| d.invisible()).child(dot(t.warn, true, &t)).child("unsaved"));
        // A list kept over a lost connection: what's wrong, above it.
        let problem_line = c.problem(&self.env).map(|p| div().flex_none().px_3().pb_2().child(icon_text(Icon::Warning, t.warn, p.to_string(), &t)));
        // Its app's repository: the branch checked out, and the running tag's branch on the
        // remote (its last commit, and when the remote was last fetched).
        let repo_text = match (&self.repo, &self.unlinked) {
            (Some((id, repo)), _) => Some(match repo.read(cx).snapshot() {
                None => format!("app {id} · reading its repository…"),
                Some(s) => {
                    let now = c.now_s();
                    let head = match &s.status.head {
                        devdeck_core::git::Head::Branch(b) => format!("on {b} locally"),
                        devdeck_core::git::Head::Detached(commit) => format!("detached at {commit} locally"),
                        devdeck_core::git::Head::Unborn(b) => format!("on {b} (no commit) locally"),
                    };
                    let tag = d.tag();
                    let remote = s.branches.iter().find(|b| b.remote.as_ref().is_some_and(|r| b.name == format!("{r}/{tag}")));
                    let running = match remote {
                        Some(b) => format!("{}: last commit {} ago", b.name, fmt_age(now.saturating_sub(b.at))),
                        None => format!("no branch {tag} on the remote"),
                    };
                    let fetched = s.fetched.map_or_else(|| "never fetched".to_string(), |f| format!("fetched {} ago", fmt_age(now.saturating_sub(f))));
                    format!("app {id} · {head} · {running} · {fetched}")
                }
            }),
            (None, Some(why)) => Some(why.clone()),
            (None, None) => None,
        };
        let repo_line = repo_text.map(|text| div().flex_none().px_3().pb_1().child(label(text).text_color(t.dim)).debug_selector(|| "svc-repo".into()));

        let rollout_line = rollout.map(|r| {
            let who = if c.mine(&r.by, cx) { String::new() } else { format!(" (by {})", r.by) };
            let (mark, text, color) = match &r.state {
                RolloutState::Rolling { ready, of } if r.redeploy() => (Icon::Busy, format!("redeploying {}{who}: {ready}/{of} pods ready", r.to), t.accent),
                RolloutState::Failing { ready, of, reason } if r.redeploy() => (Icon::Error, format!("redeploy of {}{who} failing ({ready}/{of} ready): {reason}", r.to), t.err),
                RolloutState::Stuck { reason } if r.redeploy() => (Icon::Error, format!("redeploy of {}{who} stuck: {reason}", r.to), t.err),
                RolloutState::Done if r.redeploy() => (Icon::Ok, format!("redeployed {}{who}", r.to), t.ok),
                RolloutState::Replaced { by } if r.redeploy() => (Icon::Info, format!("redeploy of {} replaced by {by}", r.to), t.mute),
                RolloutState::Rolling { ready, of } => (Icon::Busy, format!("rolling out {} → {}{who}: {ready}/{of} pods ready", r.from, r.to), t.accent),
                RolloutState::Failing { ready, of, reason } => (Icon::Error, format!("rollout to {}{who} failing ({ready}/{of} ready): {reason}", r.to), t.err),
                RolloutState::Stuck { reason } => (Icon::Error, format!("rollout to {}{who} stuck: {reason}", r.to), t.err),
                RolloutState::Done => (Icon::Ok, format!("now running {} (was {}){who}", r.to, r.from), t.ok),
                RolloutState::Replaced { by } => (Icon::Info, format!("rollout to {} replaced by {by}", r.to), t.mute),
            };
            div().flex_none().px_3().pb_2().flex().items_center().gap_2().debug_selector(|| "svc-rollout".into()).child(icon_text(mark, color, text, &t).flex_1())
        });

        let res = d.main().resources;
        let (cpu, mem) = pods.iter().filter(|p| p.ready).filter_map(usage).fold((0u32, 0u32), |(c, m), u| (c + u.cpu_m, m + u.mem_mi));
        let n = d.replicas.max(1);
        let times = |v: Option<u32>| v.map(|v| v * n);
        let metrics_el = match metrics {
            Err(e) => icon_text(Icon::Warning, t.warn, e, &t).into_any_element(),
            Ok(()) if !c.usage_read(&self.env) => label("cpu and memory: not read yet").text_color(t.dim).into_any_element(),
            Ok(()) => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().w(t.size(40.)).text_color(t.mute).child("cpu"))
                        .child(bar(cpu, times(res.cpu_request_m), times(res.cpu_limit_m), &t))
                        .child(label(format!("{cpu}m · request {} · limit {}", quantity(times(res.cpu_request_m), "m"), quantity(times(res.cpu_limit_m), "m"))).text_color(t.mute)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().w(t.size(40.)).text_color(t.mute).child("mem"))
                        .child(bar(mem, times(res.mem_request_mi), times(res.mem_limit_mi), &t))
                        .child(label(format!("{mem}Mi · request {} · limit {}", quantity(times(res.mem_request_mi), "Mi"), quantity(times(res.mem_limit_mi), "Mi"))).text_color(t.mute)),
                )
                .into_any_element(),
        };

        let col = |w: f32, text: String, color: Hsla| div().flex_none().w(t.size(w)).truncate().text_color(color).child(text);
        let mut pod_rows = div().flex().flex_col().child(
            div()
                .h(t.row_h)
                .flex()
                .items_center()
                .gap_2()
                .text_color(t.dim)
                .child(col(300., "pod".into(), t.dim))
                .child(col(150., "status".into(), t.dim))
                .child(col(60., "ready".into(), t.dim))
                .child(col(70., "restarts".into(), t.dim))
                .child(col(50., "age".into(), t.dim))
                .child(col(60., "cpu".into(), t.dim))
                .child(col(60., "mem".into(), t.dim))
                .child(col(100., "tag".into(), t.dim))
                .child(div().flex_1().min_w_0().truncate().child("node · last termination")),
        );
        match &pods_state {
            Remote::Loading => pod_rows = pod_rows.child(label("loading pods…").text_color(t.dim)),
            Remote::Error(why) => pod_rows = pod_rows.child(icon_text(Icon::Warning, t.warn, why.clone(), &t)),
            Remote::Ready(list) => {
                // A watch that failed after listing: its last pods, with what's wrong.
                if let Some(why) = c.pods_problem(&self.env) {
                    pod_rows = pod_rows.child(icon_text(Icon::Warning, t.warn, why.to_string(), &t));
                }
                if list.is_empty() {
                    pod_rows = pod_rows.child(label("no pods").text_color(t.dim));
                }
            }
        }
        let now = c.now_s();
        for (i, p) in pods.iter().enumerate() {
            let (phase, ok) = phase_text(p);
            let addr = ViewAddr::Pod { env: self.env.clone(), pod: p.name.clone() };
            let ctx = self.ctx.clone();
            let hover = t.hover;
            pod_rows = pod_rows.child(
                div()
                    .id(("pod", i))
                    .h(t.row_h)
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(t.radius_s)
                    .hover(move |s| s.bg(hover))
                    .cursor_pointer()
                    .on_mouse_down(gpui::MouseButton::Right, {
                        let (ctx, pod, logs) = (self.ctx.clone(), addr.clone(), ViewAddr::Logs { env: self.env.clone(), name: self.name.clone() });
                        move |e, window, cx| {
                            let (c1, c2) = (ctx.clone(), ctx.clone());
                            let (pod, logs) = (pod.clone(), logs.clone());
                            let pod_name = match &pod {
                                ViewAddr::Pod { pod, .. } => pod.clone(),
                                _ => String::new(),
                            };
                            open_menu(
                                e.position,
                                vec![
                                    MenuItem::new("Open pod", move |_, cx| c1.open(pod.clone(), cx)),
                                    MenuItem::new("Logs", move |_, cx| super::logs_view::open_for_pod(&c2, logs.clone(), &pod_name, cx)),
                                ],
                                window,
                                cx,
                            );
                        }
                    })
                    .on_click(move |_, _, cx| ctx.open(addr.clone(), cx))
                    .child(col(300., p.name.clone(), t.fg))
                    .child(col(150., phase, if ok { t.ok } else { t.err }))
                    .child(div().flex_none().w(t.size(60.)).child(if p.ready { icon(Icon::Check, t.ok, &t) } else { icon(Icon::Close, t.err, &t) }))
                    .child(col(70., p.restarts.to_string(), if p.restarts > 3 { t.warn } else { t.mute }))
                    .child(col(50., p.started.map_or_else(|| "—".into(), |s| fmt_age(now.saturating_sub(s))), t.mute))
                    .child(col(60., usage(p).map_or_else(|| "—".into(), |u| format!("{}m", u.cpu_m)), t.mute))
                    .child(col(60., usage(p).map_or_else(|| "—".into(), |u| format!("{}Mi", u.mem_mi)), t.mute))
                    .child(col(100., p.tag.clone(), t.mute))
                    .child(label(format!("{}{}", p.node, p.last_termination.as_ref().map(|l| format!(" · {}{}", l.text(), l.at.map(|at| format!(" {} ago", fmt_age(now.saturating_sub(at)))).unwrap_or_default())).unwrap_or_default())).flex_1().text_color(if p.last_termination.is_some() { t.warn } else { t.dim })),
            );
        }

        let mut containers = div().flex().flex_col().gap_1();
        for k in &d.containers {
            let r = k.resources;
            let list = |v: &[String]| if v.is_empty() { "none".to_string() } else { v.join(", ") };
            let ports: Vec<String> = k.ports.iter().map(u16::to_string).collect();
            containers = containers.child(
                div()
                    .flex()
                    .flex_col()
                    .debug_selector({
                        let id = format!("container-{}", k.name);
                        move || id.clone()
                    })
                    .child(label(format!("container {} · image {}", k.name, k.image)).text_color(t.fg))
                    .child(label(format!("ports {}", list(&ports))).text_color(t.mute))
                    .child(
                        label(format!(
                            "request {} / {} · limit {} / {}",
                            quantity(r.cpu_request_m, "m"),
                            quantity(r.mem_request_mi, "Mi"),
                            quantity(r.cpu_limit_m, "m"),
                            quantity(r.mem_limit_mi, "Mi")
                        ))
                        .text_color(t.mute),
                    )
                    .child(label(format!("configmaps {}", list(&k.config_maps))).text_color(t.mute)),
            );
        }

        let mut hist = div().flex().flex_col();
        if history.is_empty() {
            hist = hist.child(label("no deploy seen yet").text_color(t.dim));
        }
        let now = c.now_s();
        for r in history.iter().rev() {
            let bad = r.outcome.starts_with("failing") || r.outcome.starts_with("stuck");
            let outcome = if r.outcome.is_empty() { "rolling out" } else { r.outcome.as_str() };
            let what = match (r.redeploy(), r.outcome.is_empty()) {
                (true, true) => format!("redeploying {}", r.to),
                (true, false) => format!("redeployed {}", r.to),
                (false, _) => format!("{} → {}", r.from, r.to),
            };
            let text = format!("{} ago · {what} · by {} · {outcome}", fmt_age(now.saturating_sub(r.at_s)), r.by);
            hist = hist.child(label(text).text_color(if bad { t.err } else { t.mute }));
        }

        let body = div()
            .id("service-body")
            .flex_1()
            .min_h_0()
            .scroll_y()
            .px_3()
            .pb_3()
            .flex()
            .flex_col()
            .gap_1()
            .child(section("Metrics", &t))
            .child(metrics_el)
            .child(section("Pods", &t))
            // Wider than a narrow pane: scrolls sideways instead of clipping (R3.9).
            .child(div().id("pods-scroll").w_full().scroll_x().child(pod_rows.min_w(t.size(1100.))))
            .child(section(if d.containers.len() > 1 { "Containers" } else { "Container" }, &t))
            .child(containers)
            .child(section("Deploy history", &t))
            .child(hist);

        self.window = Some(window.window_handle());
        root.child(header).child(actions).child(editor).children(repo_line).children(problem_line).children(rollout_line).child(body)
    }
}

// ------------------------------------------------------------------ modals

/// The write a confirmation sends. The image shown is what the write is pinned to: it is
/// refused if the cluster changed since.
#[derive(Clone, Debug, PartialEq)]
pub enum WriteKind {
    /// Another branch: the tags `from` → `to`, and the image running when confirming.
    Change { from: String, from_image: String, to: String },
    /// The same branch again (`kubectl rollout restart`), what may keep the old build.
    Redeploy { tag: String, image: String, warning: Option<String> },
}

/// "Deploy X on env: from → to" (or "Redeploy X on env?" with its branch) with the
/// equivalent command; Enter sends, Esc cancels. It stays until the cluster answers: closes
/// when the write is taken, shows why when not.
pub struct ConfirmDeploy {
    service: Entity<ServiceView>,
    env: String,
    name: String,
    pub kind: WriteKind,
    command: String,
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
    /// Sent: waiting for the answer.
    sent: bool,
    error: Option<String>,
    _cluster: Option<Subscription>,
}

impl ConfirmDeploy {
    fn new(service: Entity<ServiceView>, env: String, name: String, kind: WriteKind, command: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let restore = window.focused(cx).map(|f| f.downgrade());
        let cluster = service.read(cx).cluster.clone();
        let sub = cx.observe_in(&cluster, window, |this: &mut Self, c, window, cx| {
            if !this.sent {
                return;
            }
            match c.read(cx).write_state(&this.env, &this.name).cloned() {
                Some(WriteState::Sent) => {
                    c.update(cx, |c, _| c.clear_write(&this.env, &this.name));
                    this.close(window, cx);
                }
                Some(WriteState::Failed(why)) => {
                    c.update(cx, |c, _| c.clear_write(&this.env, &this.name));
                    this.sent = false;
                    this.error = Some(why);
                    cx.notify();
                }
                _ => {}
            }
        });
        Self { service, env, name, kind, command, focus: ModalFocus(vec![cx.focus_handle(), cx.focus_handle()]), restore, sent: false, error: None, _cluster: Some(sub) }
    }

    /// Why the cluster refused the write, when it did (shown in it).
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Sends the write; the modal waits for the answer.
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sent {
            return;
        }
        let cluster = self.service.read(cx).cluster.clone();
        cluster.update(cx, |c, _| c.watch_write(&self.env, &self.name, true));
        let kind = self.kind.clone();
        let sent = self.service.update(cx, |s, cx| match &kind {
            WriteKind::Change { from_image, to, .. } => s.deploy(from_image, to, cx),
            WriteKind::Redeploy { image, .. } => s.send_redeploy(image, cx),
        });
        match sent {
            Ok(()) => {
                self.sent = true;
                self.error = None;
            }
            Err(why) => self.error = Some(why),
        }
        let _ = window;
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Closed while sending: the answer goes to a toast.
        let cluster = self.service.read(cx).cluster.clone();
        cluster.update(cx, |c, _| c.watch_write(&self.env, &self.name, false));
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.0[1].is_focused(window) {
            return self.close(window, cx);
        }
        self.send(window, cx);
    }
}

impl HasModalFocus for ConfirmDeploy {
    fn modal_focus(&self) -> &ModalFocus {
        &self.focus
    }
}

fn focus_button(id: &'static str, text: &'static str, handle: &FocusHandle, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let ring = t.focus;
    button(id, text, t).border_1().border_color(t.line).track_focus(handle).focus(move |s| s.border_color(ring))
}

impl Render for ConfirmDeploy {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let redeploy = matches!(self.kind, WriteKind::Redeploy { .. });
        let warning = match &self.kind {
            WriteKind::Redeploy { warning, .. } => warning.clone(),
            WriteKind::Change { .. } => None,
        };
        let content = div()
            .key_context("Modal ServiceModal")
            .flex()
            .flex_col()
            .gap_2()
            .max_w(t.size(760.))
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &ConfirmModal, window, cx| this.confirm(window, cx)))
            .child(div().text_color(t.hi).child(format!("{} {} on {}?", if redeploy { "Redeploy" } else { "Deploy" }, self.name, self.env)))
            .child(match &self.kind {
                WriteKind::Change { from, to, .. } => div().flex().items_center().gap_2().child(label(from.clone()).text_color(t.mute)).child(icon(Icon::To, t.mute, &t)).child(label(to.clone()).text_color(t.hi)),
                WriteKind::Redeploy { tag, .. } => div().flex().items_center().gap_2().child(label("branch").text_color(t.mute)).child(label(tag.clone()).text_color(t.hi)),
            })
            .child(label(if redeploy { "Equivalent of:" } else { "Runs:" }).text_color(t.mute))
            // The one place text wraps (R4.4 allows a view to opt in): the command is
            // shown whole, so the tag being deployed is always visible.
            .child(div().px_2().py_1().rounded(t.radius_s).bg(t.bg).text_color(t.fg).debug_selector(|| "deploy-command".into()).child(self.command.clone()))
            // Wraps: the whole of what the redeploy may not do.
            .when_some(warning, |d, w| d.child(div().flex().gap_1().text_color(t.warn).debug_selector(|| "redeploy-warning".into()).child(icon(Icon::Warning, t.warn, &t)).child(div().flex_1().min_w_0().child(w))))
            .when(self.sent, |d| d.child(icon_text(Icon::Busy, t.accent, "sending…", &t)))
            // Wraps too: the cluster's words, whole.
            .when_some(self.error.clone(), |d, why| d.child(div().flex().gap_1().text_color(t.err).debug_selector(|| "deploy-error".into()).child(icon(Icon::Error, t.err, &t)).child(div().child(why))))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(focus_button("deploy-cancel", "Cancel", &self.focus.0[1], &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))))
                    .child(
                        focus_button("deploy-confirm", if redeploy { "Redeploy" } else { "Deploy" }, &self.focus.0[0], &t)
                            .debug_selector(|| "deploy-confirm".into())
                            .when(self.sent, |b| b.text_color(t.dim))
                            .on_click(cx.listener(|this, _, w, cx| this.send(w, cx))),
                    ),
            );
        modal_frame(content.into_any_element(), window, &t)
    }
}

/// Unlocks a protected env for one view: type its exact name.
pub struct UnlockEnv {
    service: Entity<ServiceView>,
    env: String,
    input: Entity<TextInput>,
    error: Option<SharedString>,
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
    _sub: Subscription,
}

impl UnlockEnv {
    fn new(service: Entity<ServiceView>, env: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let restore = window.focused(cx).map(|f| f.downgrade());
        let input = cx.new(|cx| TextInput::new("type the env name", cx));
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| {
            if let InputEvent::Submit(_) = e {
                this.submit(window, cx);
            }
        });
        let handles = vec![input.read(cx).focus_handle(cx), cx.focus_handle()];
        Self { service, env, input, error: None, focus: ModalFocus(handles), restore, _sub: sub }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.input.read(cx).text().to_string();
        let ok = self.service.read(cx).cluster.read(cx).env(&self.env).is_some_and(|e| unlock_matches(e, &typed));
        if ok {
            self.close(window, cx);
            self.service.update(cx, |s, cx| {
                s.unlocked = true;
                cx.notify();
            });
        } else {
            self.error = Some(format!("That isn't {}. Type it exactly.", self.env).into());
            cx.notify();
        }
    }
}

impl HasModalFocus for UnlockEnv {
    fn modal_focus(&self) -> &ModalFocus {
        &self.focus
    }
}

impl Render for UnlockEnv {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let content = div()
            .key_context("Modal")
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.close(window, cx)))
            .child(icon_text(Icon::Protected, t.err, format!("{} is protected", self.env), &t))
            .child(label(format!("Type {} to allow writes from this view until it closes.", self.env)).text_color(t.mute))
            .child(div().w(t.size(320.)).child(self.input.clone()))
            .children(self.error.clone().map(|e| label(e).text_color(t.err)))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(focus_button("unlock-cancel", "Cancel", &self.focus.0[1], &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx)))),
            );
        modal_frame(content.into_any_element(), window, &t)
    }
}
