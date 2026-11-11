//! Service page (`svc/<env>/<name>`, R7.2–R7.6, R7.8): one page of panels, its title the
//! header's (a tab's, Cluster's details). Deployment: its facts (state, tag, pods, when and
//! by whom, revision, usage, the protected env), the rollout under way, then Deploy… (the
//! picker, then the confirmation stating the exact command, waiting for the cluster's
//! answer), Redeploy, the notify dropdown and the lock, together. Pods: a table whose rows
//! open in place (node, last end, containers, init ones first). Logs: the service's logs
//! view, in it. Envs: its tag and pods on each, a row shows that env. History, Containers.
//! Opening it asks the cluster whether the account may change deployments there.
//!
//! Metrics are sampled only while the view is visible (it holds a [`Sampling`] handle).
//! Keys (`ServiceView`): ctrl+s deploy, l logs on their own, r redeploy, b notify. What the
//! notify dropdown ticks is the cluster's ([`crate::cluster_notes`]): it outlives the view.

use super::logs_view::ServiceLogsView;
use super::{ViewCx, fmt_age};
use crate::sidebar::Activity;
use crate::cluster::{Cluster, PodScope, PodsInterest, Sampling, WriteState};
use crate::danger::{self, Confirmed, DangerModal};
use crate::repo::{Looking, Repo};
use devdeck_core::cluster::{Deployment, Pod, PodPhase, Remote, RolloutState, redeploy_warning, unlock_matches};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::Scrolls;
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{
    Column, Icon, InputEvent, MenuItem, ModalFocus, PickGroup, PickOption, PickerSpec, PodMark, TextInput, Theme, ToastKind, button, dot, icon, icon_action, icon_text,
    label, modal_frame, notify_button, open_menu, open_picker, pop_modal, push_modal, table_head, table_more, table_row, theme, tip, toasts, under,
};
use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;
use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, Pixels, Context, Entity, FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, WeakFocusHandle, Window, actions, div, prelude::*,
};

actions!(service_view, [SaveTag, OpenLogs, Redeploy, NotifyMenu, ConfirmModal, OpenApp]);

/// Its header's action (a tab's, Cluster's details): the registry's id.
pub const OPEN_APP: &str = "svc.app";

pub struct ServiceView {
    pub env: String,
    pub name: String,
    ctx: ViewCx,
    cluster: Entity<Cluster>,
    focus: FocusHandle,
    pub tag: Entity<TextInput>,
    /// The part the tag goes to ([`Deployment::part`]: an app inside it); `None`: itself.
    pub part: Option<String>,
    /// The tag shown when the editor isn't dirty.
    shown_tag: Option<String>,
    /// Writes allowed on a protected env until the view closes (R7.6).
    pub unlocked: bool,
    sampling: Option<Sampling>,
    /// Its pods are watched while it's open.
    _pods: PodsInterest,
    /// The app it's deployed from, once linked, and its repository (M3 step 3).
    app: Option<String>,
    repo: Option<(String, Entity<Repo>)>,
    /// Its logs, in the page (read while it shows, as a `logs/…` tab would).
    pub logs: Entity<ServiceLogsView>,
    /// The pods whose row is open.
    pub open_pods: BTreeSet<String>,
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
        let mut subs = vec![
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
            // Its Envs panel: every env read, as the matrix does.
            c.want_all(cx);
            c.pods_interest(&env, PodScope::Service(name.clone()), cx)
        });
        let logs = {
            let (env, name, ctx, cluster) = (env.clone(), name.clone(), ctx.clone(), cluster.clone());
            cx.new(|cx| ServiceLogsView::new(env, name, ctx, cluster, cx))
        };
        let mut this = Self {
            env,
            name,
            ctx,
            app: None,
            logs,
            open_pods: BTreeSet::new(),
            cluster,
            focus: cx.focus_handle(),
            tag,
            part: None,
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
        let apps = self.ctx.services.read(cx).catalog.apps.clone();
        let found = devdeck_core::links::app_for(&d.name, &image, &apps);
        // Serving apps of its own (its parts): their repository, for their branches.
        let part = d.init_containers.iter().find_map(|k| devdeck_core::links::part_app(&k.name, &k.image.raw, &apps));
        let id = match found {
            Ok(Some(id)) => id,
            Ok(None) if let Some(part) = part => {
                // Its parts' repository, when there's one; it isn't the page's own to explain.
                self.link_repo(part, cx);
                self.unlinked = None;
                return;
            }
            Ok(None) => {
                self.unlinked = Some(format!("no app of this workspace deploys as {}: add `deployments = [\"{}\"]` to its app in the workspace file", d.name, d.name));
                return;
            }
            Err(problem) => {
                self.unlinked = Some(problem);
                return;
            }
        };
        if self.app.as_ref() != Some(&id) {
            self.app = Some(id.clone());
            self.unlinked = None;
            cx.notify();
        }
        self.link_repo(id, cx);
    }

    /// Reads `id`'s repository while the page shows (its branches for Deploy…).
    fn link_repo(&mut self, id: String, cx: &mut Context<Self>) {
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

    /// Its app in Apps.
    fn open_app(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.app.clone() {
            self.ctx.pivot(Activity::Apps, id, None, cx);
        }
    }

    /// Opens or closes a pod's row.
    pub fn toggle_pod(&mut self, pod: &str, cx: &mut Context<Self>) {
        if !self.open_pods.remove(pod) {
            self.open_pods.insert(pod.to_string());
        }
        cx.notify();
    }

    /// Its logs panel shows `pod`'s lines only (`None`: every pod's).
    pub fn logs_of(&mut self, pod: Option<SharedString>, cx: &mut Context<Self>) {
        self.logs.update(cx, |l, cx| l.set_pod(pod, cx));
    }

    /// A pod row's menu: the pod on its own, its lines in the logs panel.
    fn pod_menu(this: &Entity<Self>, pod: &str, cx: &App) -> Vec<MenuItem> {
        let s = this.read(cx);
        let (ctx, addr) = (s.ctx.clone(), ViewAddr::Pod { env: s.env.clone(), pod: pod.to_string() });
        let (me, name) = (this.clone(), SharedString::from(pod.to_string()));
        vec![
            MenuItem::new("Open pod", move |_, cx| ctx.open(addr.clone(), cx)),
            MenuItem::new("Logs", move |_, cx| me.update(cx, |s, cx| s.logs_of(Some(name.clone()), cx))),
        ]
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
        // The short name first: it's what the user can do something about.
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
        if let Some(part) = self.part.clone() {
            return self.save_part(&part, cx);
        }
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
            let kind = WriteKind::Change { from: from_tag, from_image, to, part: None };
            let modal = cx.new(|cx| ConfirmDeploy::new(me, env, name, kind, command, window, cx));
            danger::show(modal, window, cx);
        });
    }

    /// Deploys the tag typed or picked to `part`: as [`ServiceView::save`], its write
    /// targeting that part's init container only.
    fn save_part(&mut self, part: &str, cx: &mut Context<Self>) {
        let to = self.tag.read(cx).text().trim().to_string();
        let other = self.cluster.read(cx).part_blocked(&self.env, &self.name, part, &to, true, cx);
        if other.is_none() && self.protected(cx) && !self.unlocked {
            self.request_unlock(cx);
            return;
        }
        if let Some(why) = self.cluster.read(cx).part_blocked(&self.env, &self.name, part, &to, self.unlocked, cx) {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("Not deployed: {why}."), vec![], cx));
            return;
        }
        let c = self.cluster.read(cx);
        let (Some(d), Some(command)) = (c.deployment(&self.env, &self.name), c.part_command_for(&self.env, &self.name, part, &to)) else { return };
        let Some((_, k)) = d.part(part) else { return };
        let kind = WriteKind::Change { from: k.image.label(), from_image: k.image.raw.clone(), to, part: Some(part.to_string()) };
        let me = cx.entity();
        let (env, name) = (self.env.clone(), self.name.clone());
        self.in_window(cx, move |window, cx| {
            let modal = cx.new(|cx| ConfirmDeploy::new(me, env, name, kind, command, window, cx));
            danger::show(modal, window, cx);
        });
    }

    /// Sends a part's change confirmed: `to` on its image `from` that was shown.
    pub fn deploy_part(&mut self, part: &str, from: &str, to: &str, ok: Confirmed, cx: &mut Context<Self>) -> Result<(), String> {
        let (env, name, unlocked) = (self.env.clone(), self.name.clone(), self.unlocked);
        let result = self.cluster.update(cx, |c, cx| c.deploy_part(&env, &name, part, from, to, unlocked, ok, cx));
        cx.notify();
        result
    }

    /// Why a redeploy can't go ahead now, if it can't (every guard of a write).
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
            danger::show(modal, window, cx);
        });
    }

    /// Sends the redeploy confirmed, pinned to the image that was shown.
    pub fn send_redeploy(&mut self, image: &str, ok: Confirmed, cx: &mut Context<Self>) -> Result<(), String> {
        let (env, name, unlocked) = (self.env.clone(), self.name.clone(), self.unlocked);
        let result = self.cluster.update(cx, |c, cx| c.redeploy(&env, &name, image, unlocked, ok, cx));
        cx.notify();
        result
    }

    /// Sends the change confirmed: `to` on the image `from` that was shown.
    pub fn deploy(&mut self, from: &str, to: &str, ok: Confirmed, cx: &mut Context<Self>) -> Result<(), String> {
        let (env, name, unlocked) = (self.env.clone(), self.name.clone(), self.unlocked);
        let result = self.cluster.update(cx, |c, cx| c.deploy(&env, &name, from, to, unlocked, ok, cx));
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

    /// Deploy…: the picker of what can go here (the tags on the other envs, its app's
    /// branches on the remote, the tags it ran before, or what's typed); a pick goes on to
    /// the checks and the confirmation, as the rocket did ([`ServiceView::save`]).
    pub fn open_deploy(this: &Entity<Self>, window: &mut Window, cx: &mut App) {
        Self::open_deploy_part(this, None, window, cx);
    }

    /// Deploy… for `part` (an app inside it, [`Deployment::part`]; `None`: itself): its
    /// tags on the other envs, its repository's remote branches, the tags it ran here
    /// before (itself only), or what's typed.
    pub fn open_deploy_part(this: &Entity<Self>, part: Option<String>, window: &mut Window, cx: &mut App) {
        let s = this.read(cx);
        let c = s.cluster.read(cx);
        let now = c.now_s();
        // A version (`1.62.8`, `v2.27.4`) is a tag; anything else, a branch's.
        let kind = |t: &str| if t.trim_start_matches('v').starts_with(|c: char| c.is_ascii_digit()) { "tag" } else { "branch" };
        let (mut envs, mut before) = (Vec::new(), Vec::new());
        let running = match &part {
            None => s.shown_tag.clone(),
            Some(p) => c.deployment(&s.env, &s.name).and_then(|d| d.part(p).map(|(_, k)| k.image.label())),
        };
        match &part {
            None => {
                for (tag, why) in c.tag_suggestions(&s.env, &s.name) {
                    match why.strip_prefix("on ") {
                        Some(env) => envs.push(PickOption::new(tag.clone()).chip(env.to_string()).kind(kind(&tag))),
                        None => before.push(PickOption::new(tag.clone()).kind(kind(&tag))),
                    }
                }
            }
            // The part's tag on each other env, once each.
            Some(p) => {
                for e in c.envs.iter().filter(|e| e.name != s.env) {
                    let Some(tag) = c.deployment(&e.name, &s.name).and_then(|d| d.part(p).map(|(_, k)| k.image.label())) else { continue };
                    if Some(&tag) != running.as_ref() && !envs.iter().any(|o: &PickOption| o.value == tag) {
                        envs.push(PickOption::new(tag.clone()).chip(e.name.clone()).kind(kind(&tag)));
                    }
                }
            }
        }
        let mut branches = Vec::new();
        // Its app's repository (a part's: the one its apps are in); none for a deployment
        // linked to no app of its own.
        let repo = s.repo.as_ref().filter(|_| part.is_some() || s.app.is_some());
        if let Some(snapshot) = repo.and_then(|(_, r)| r.read(cx).snapshot()) {
            // Remote branches only: a branch only local was never built.
            for b in snapshot.branches.iter().filter(|b| b.remote.is_some()) {
                let remote = b.remote.as_deref().unwrap_or_default();
                let name = b.name.strip_prefix(&format!("{remote}/")).unwrap_or(&b.name).to_string();
                if Some(&name) != running.as_ref() {
                    branches.push(PickOption::new(name).detail(format!("{} ago · {}", fmt_age(now.saturating_sub(b.at)), b.author)).kind("branch"));
                }
            }
        }
        let groups = [("ON OTHER ENVS", envs), ("BRANCHES", branches), ("BEFORE HERE", before)]
            .into_iter()
            .filter(|(_, o)| !o.is_empty())
            .map(|(label, options)| PickGroup { label: label.into(), options })
            .collect();
        let me = this.clone();
        let title = match &part {
            None => format!("Deploy {} to {}", s.name, s.env),
            Some(p) => format!("Deploy {p} in {} to {}", s.name, s.env),
        };
        let spec = PickerSpec {
            title: title.into(),
            placeholder: "search branches and tags".into(),
            kinds: vec!["branch", "tag"],
            groups,
            typed: Some("as typed".into()),
            on_pick: Rc::new(move |tag, _, cx| {
                me.update(cx, |s, cx| {
                    s.tag.update(cx, |i, cx| i.set_text(&tag, cx));
                    s.part = part.clone();
                    s.save(cx);
                })
            }),
        };
        open_picker(spec, window, cx);
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

/// A pod in a word, colored by how much it matters: what a row of the pods says.
fn pod_state(p: &Pod, t: &Theme) -> (String, Hsla) {
    match &p.phase {
        PodPhase::Running if p.ready => ("running".into(), t.ok),
        PodPhase::Running => ("starting".into(), t.warn),
        PodPhase::Pending => ("pending".into(), t.warn),
        PodPhase::Terminating => ("terminating".into(), t.mute),
        PodPhase::Succeeded => ("ended".into(), t.mute),
        PodPhase::Waiting(r) if p.trouble().is_none() => (r.clone(), t.warn),
        PodPhase::Waiting(r) | PodPhase::Failed(r) => (r.clone(), t.err),
    }
}

/// A pod's square, as its state says.
fn pod_mark(p: &Pod) -> PodMark {
    match &p.phase {
        _ if p.ready => PodMark::Ready,
        PodPhase::Failed(_) => PodMark::Failing,
        PodPhase::Waiting(_) if p.trouble().is_some() => PodMark::Failing,
        PodPhase::Terminating | PodPhase::Succeeded => PodMark::Old,
        _ => PodMark::Starting,
    }
}

/// The pods' table: the name takes the room left, the data columns don't move.
const POD_COLS: [Column; 6] =
    [Column::rest("POD"), Column::fixed("STATE", 110.), Column::fixed("AGE", 50.), Column::fixed("RESTARTS", 64.).right(), Column::fixed("CPU", 64.), Column::fixed("MEMORY", 70.)];
/// A pod's containers, under its row.
const CONTAINER_COLS: [Column; 3] = [Column::rest("CONTAINER"), Column::fixed("STATE", 220.), Column::fixed("RESTARTS", 64.).right()];
/// Its parts: the app, its tag, its Deploy…, a way to the app.
const PART_COLS: [Column; 4] = [Column::rest("APP"), Column::fixed("TAG", 170.), Column::fixed("", 100.).right(), Column::fixed("", 24.)];
const ENV_COLS: [Column; 3] = [Column::rest("ENV"), Column::fixed("TAG", 130.), Column::fixed("PODS", 90.)];
const HISTORY_COLS: [Column; 3] = [Column::fixed("WHEN", 56.), Column::fixed("WHO", 110.), Column::rest("CHANGE")];

/// Its header's icon: its app in Apps, once linked.
impl super::Live for ServiceView {
    fn actions(&self, _cx: &App) -> Vec<super::ViewAction> {
        self.app.as_ref().map(|_| super::ViewAction { id: OPEN_APP, icon: Icon::Apps, on: false }).into_iter().collect()
    }

    fn run(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
        if id == OPEN_APP {
            self.open_app(cx);
        }
    }
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
            // After this update: the picker reads the view.
            .on_action(cx.listener(|_, _: &SaveTag, window, cx| {
                let me = cx.entity();
                window.defer(cx, move |window, cx| ServiceView::open_deploy(&me, window, cx));
            }))
            .on_action(cx.listener(|this, _: &OpenLogs, _, cx| this.open_logs(cx)))
            .on_action(cx.listener(|this, _: &Redeploy, _, cx| this.redeploy(cx)))
            .on_action(cx.listener(|this, _: &OpenApp, _, cx| this.open_app(cx)))
            .on_action(cx.listener(|this, _: &NotifyMenu, window, cx| this.open_notify(window, cx)));
        let message = |text: String, color: Hsla| div().flex_1().flex().items_center().justify_center().text_color(color).child(label(text));
        let d = match (&state, c.deployment(&self.env, &self.name)) {
            (Remote::Loading, _) => return root.child(message(format!("loading {}…", self.env), t.mute)),
            (Remote::Error(e), _) => {
                return root.child(div().flex_1().flex().items_center().justify_center().child(icon_text(Icon::Warning, t.warn, e.to_string(), &t)));
            }
            (Remote::Ready(_), None) => return root.child(message(format!("{} isn't deployed in {}.", self.name, self.env), t.mute)),
            (Remote::Ready(_), Some(d)) => d.clone(),
        };
        let now = c.now_s();
        let rollout = c.rollout(&self.env, &self.name).cloned();
        let pods_state = c.pods_state(&self.env, &self.name);
        let pods: Vec<Pod> = pods_state.ready().map(|p| p.to_vec()).unwrap_or_default();
        let usage = |p: &Pod| c.usage(&self.env, &p.name);
        let history = c.history(&self.env, &self.name).to_vec();
        let redeploy_blocked = self.redeploy_blocked(cx);
        let small = t.icon_small;

        // ---- Deployment: its facts, the rollout under way, then what can be done with it.
        let (mark, word, color) = match rollout.as_ref().map(|r| &r.state) {
            Some(RolloutState::Rolling { .. }) => (icon(Icon::Busy, t.accent, &t).size(small).into_any_element(), "rolling out", t.accent),
            Some(RolloutState::Failing { .. }) => (icon(Icon::Error, t.err, &t).size(small).into_any_element(), "failing", t.err),
            Some(RolloutState::Stuck { .. }) => (icon(Icon::Error, t.err, &t).size(small).into_any_element(), "stuck", t.err),
            _ if d.replicas == 0 => (dot(t.dim, false, &t).into_any_element(), "stopped", t.mute),
            _ if d.ready == d.replicas => (dot(t.ok, true, &t).into_any_element(), "running", t.ok),
            _ => (icon(Icon::Warning, t.warn, &t).size(small).into_any_element(), "not ready", t.warn),
        };
        let row = || div().flex().items_center().gap_1().min_w_0();
        let marks: Vec<PodMark> = if pods.is_empty() { super::app_view::pod_marks(&d) } else { pods.iter().map(pod_mark).collect() };
        let mut facts: Vec<(SharedString, AnyElement)> = vec![
            ("State".into(), row().debug_selector(|| "svc-state".into()).text_color(color).child(mark).child(word).into_any_element()),
            ("Tag".into(), label(d.tag()).text_color(t.fg).into_any_element()),
            ("Pods".into(), devdeck_ui::pods_bar("svc-pods", &marks, &t).into_any_element()),
        ];
        // When it got this version, and who changed it last (devdeck's own record).
        if let Some(since) = c.since(&self.env, &d) {
            let by = history.iter().rev().find(|r| r.to == d.tag()).map(|r| r.by.clone()).filter(|b| !b.is_empty() && b != "seen");
            let text = match by {
                Some(by) => format!("{} ago · {by}", fmt_age(now.saturating_sub(since))),
                None => format!("{} ago", fmt_age(now.saturating_sub(since))),
            };
            facts.push(("Deployed".into(), label(text).text_color(t.mute).into_any_element()));
        }
        facts.push(("Revision".into(), label(d.revision.to_string()).text_color(t.mute).into_any_element()));
        // What its ready pods use, against what they ask and may take.
        let res = d.main().resources;
        let n = d.replicas.max(1);
        let times = |v: Option<u32>| v.map(|v| v * n);
        match c.metrics_allowed(&self.env) {
            Err(e) => facts.push(("Usage".into(), icon_text(Icon::Warning, t.warn, e, &t).into_any_element())),
            Ok(()) if !c.usage_read(&self.env) => {}
            Ok(()) => {
                let (cpu, mem) = pods.iter().filter(|p| p.ready).filter_map(usage).fold((0u32, 0u32), |(c, m), u| (c + u.cpu_m, m + u.mem_mi));
                let bar = |name: &'static str, used: u32, req: Option<u32>, lim: Option<u32>, unit: &str| {
                    row()
                        .id(name)
                        .gap_2()
                        .child(devdeck_ui::usage_bar(name, used, req, lim, &t))
                        .child(label(format!("{used}{unit} / {}", quantity(lim.or(req), unit))).text_color(t.mute))
                        .tooltip(tip(format!("request {} · limit {}", quantity(req, unit), quantity(lim, unit))))
                        .into_any_element()
                };
                facts.push(("CPU".into(), bar("cpu", cpu, times(res.cpu_request_m), times(res.cpu_limit_m), "m")));
                facts.push(("Memory".into(), bar("mem", mem, times(res.mem_request_mi), times(res.mem_limit_mi), "Mi")));
            }
        }
        if protected {
            let (text, color) = if self.unlocked { ("protected · unlocked", t.warn) } else { ("protected", t.err) };
            facts.push(("Env".into(), row().text_color(color).child(icon(Icon::Protected, color, &t).size(small)).child(text).into_any_element()));
        }

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
            div().px_2().py_1().flex().items_center().min_w_0().debug_selector(|| "svc-rollout".into()).child(icon_text(mark, color, text, &t).flex_1())
        });

        // Deploying's actions together: Deploy…, Redeploy, notify; the lock at the end.
        let enabled = redeploy_blocked.is_none();
        let actions = div()
            .px_2()
            .py_1()
            .flex()
            .items_center()
            .gap_1()
            .child({
                let me = me.clone();
                devdeck_ui::labeled_button("svc-deploy", Icon::Deploy, "Deploy…", &t)
                    .debug_selector(|| "svc-deploy".into())
                    .on_click(move |_, window, cx| ServiceView::open_deploy(&me, window, cx))
            })
            .child(
                devdeck_ui::labeled_button("redeploy", Icon::Restart, "Redeploy", &t)
                    .debug_selector(|| "svc-redeploy".into())
                    .when(!enabled, |b| b.text_color(t.dim))
                    .tooltip(tip(redeploy_blocked.unwrap_or_else(|| "the same tag again, new pods".into())))
                    .on_click(cx.listener(|this, _, _, cx| this.redeploy(cx))),
            )
            .child({
                let ticked = !c.notify_options(&self.env, &self.name).is_empty();
                notify_button("notify", ticked, self.notify_at.clone(), &t)
                    .debug_selector(|| "tag-notify".into())
                    .on_click(cx.listener(|this, _, window, cx| this.open_notify(window, cx)))
            })
            .when(protected, |a| {
                a.child(div().flex_1()).child(
                    devdeck_ui::labeled_button("lock", Icon::Protected, if self.unlocked { "Lock" } else { "Unlock…" }, &t).on_click(cx.listener(|this, _, _, cx| {
                        if this.unlocked {
                            this.unlocked = false;
                            cx.notify();
                        } else {
                            this.request_unlock(cx);
                        }
                    })),
                )
            });
        // Why its app isn't known (its branches aren't offered then).
        let unlinked = self.unlinked.clone().map(|why| div().px_2().py_1().min_w_0().debug_selector(|| "svc-unlinked".into()).child(icon_text(Icon::Info, t.mute, why, &t)));
        let deployment = devdeck_ui::panel(
            "svc-deployment",
            "Deployment",
            Vec::new(),
            div().flex().flex_col().min_w_0().child(devdeck_ui::fields(facts, &t)).children(rollout_line).child(actions).children(unlinked),
            &t,
        );

        // ---- Apps: the apps it serves (its parts), each deployed on its own.
        let parts = (!d.init_containers.is_empty()).then(|| {
            let apps = self.ctx.services.read(cx).catalog.apps.clone();
            let mut block = div().flex().flex_col().min_w_0().child(table_head(&PART_COLS, &t));
            for (i, k) in d.init_containers.iter().enumerate() {
                let short = k.image.short_name().to_string();
                let app = devdeck_core::links::part_app(&k.name, &k.image.raw, &apps);
                let deploy = {
                    let (me, part) = (me.clone(), short.clone());
                    devdeck_ui::labeled_button(("part-deploy", i), Icon::Deploy, "Deploy…", &t)
                        .debug_selector(move || format!("part-deploy-{i}"))
                        .on_click(move |_, window, cx| ServiceView::open_deploy_part(&me, Some(part.clone()), window, cx))
                };
                let go = match app {
                    Some(id) => {
                        let ctx = self.ctx.clone();
                        devdeck_ui::go_action(("part-app", i), Icon::Apps, format!("{id} in Apps"), &t)
                            .on_click(move |_, _, cx| ctx.pivot(Activity::Apps, id.clone(), None, cx))
                            .into_any_element()
                    }
                    None => div().into_any_element(),
                };
                block = block.child(
                    table_row(("part", i), &PART_COLS, vec![label(short).text_color(t.fg).into_any_element(), label(k.image.label()).text_color(t.mute).into_any_element(), deploy.into_any_element(), go], &t)
                        .debug_selector(move || format!("part-{i}")),
                );
            }
            devdeck_ui::panel("svc-parts", format!("Apps · {}", d.init_containers.len()), Vec::new(), block, &t)
        });

        // ---- Pods: one row each; a row opens in place on what it's made of.
        let mut pod_block = div().flex().flex_col().min_w_0();
        let note = |el: AnyElement| div().px_2().h(t.row_h).flex().items_center().min_w_0().child(el);
        match &pods_state {
            Remote::Loading => pod_block = pod_block.child(note(label("loading…").text_color(t.dim).into_any_element())),
            Remote::Error(why) => pod_block = pod_block.child(note(icon_text(Icon::Warning, t.warn, why.clone(), &t).into_any_element())),
            Remote::Ready(list) => {
                // A watch that failed after listing: its last pods, with what's wrong.
                if let Some(why) = c.pods_problem(&self.env) {
                    pod_block = pod_block.child(note(icon_text(Icon::Warning, t.warn, why.to_string(), &t).into_any_element()));
                }
                if list.is_empty() {
                    pod_block = pod_block.child(note(label("no pod").text_color(t.dim).into_any_element()));
                }
            }
        }
        if !pods.is_empty() {
            pod_block = pod_block.child(table_head(&POD_COLS, &t));
        }
        let prefix = format!("{}-", self.name);
        for (i, p) in pods.iter().enumerate() {
            let open = self.open_pods.contains(&p.name);
            let (word, color) = pod_state(p, &t);
            let short = p.name.strip_prefix(&prefix).unwrap_or(&p.name).to_string();
            let use_of = usage(p);
            let cells = vec![
                row()
                    .child(icon(if open { Icon::Open } else { Icon::Expand }, t.dim, &t).size(small))
                    .child(devdeck_ui::pod_square(pod_mark(p), &t))
                    .child(label(short).text_color(t.fg))
                    .into_any_element(),
                label(word).text_color(color).into_any_element(),
                label(p.started.map_or_else(|| "—".into(), |s| fmt_age(now.saturating_sub(s)))).text_color(t.mute).into_any_element(),
                label(if p.restarts > 0 { format!("↻ {}", p.restarts) } else { String::new() }).text_color(t.warn).into_any_element(),
                label(use_of.map_or_else(|| "—".into(), |u| format!("{}m", u.cpu_m))).text_color(t.mute).into_any_element(),
                label(use_of.map_or_else(|| "—".into(), |u| format!("{}Mi", u.mem_mi))).text_color(t.mute).into_any_element(),
            ];
            let name = p.name.clone();
            pod_block = pod_block.child(
                table_row(("pod", i), &POD_COLS, cells, &t)
                    .debug_selector(move || format!("pod-{i}"))
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .on_mouse_down(gpui::MouseButton::Right, {
                        let (me, pod) = (me.clone(), p.name.clone());
                        move |e, window, cx| {
                            let items = ServiceView::pod_menu(&me, &pod, cx);
                            open_menu(e.position, items, window, cx);
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_pod(&name, cx))),
            );
            if open {
                pod_block = pod_block.child(table_more(self.pod_more(p, i, now, me.clone(), &t), &t));
            }
        }
        let pods_panel = devdeck_ui::panel("svc-pods-panel", format!("Pods · {}", pods.len()), Vec::new(), pod_block, &t);

        // ---- Logs: the service's, read while the page shows; on their own from its icon.
        let open_logs = icon_action("svc-logs-open", Icon::Logs, "the logs on their own", true, &t).on_click(cx.listener(|this, _, _, cx| this.open_logs(cx)));
        let logs = devdeck_ui::panel(
            "svc-logs",
            "Logs",
            vec![open_logs.into_any_element()],
            // A fixed height: its lines scroll in it.
            div().h(t.size(320.)).min_w_0().flex().flex_col().child(self.logs.clone()),
            &t,
        );

        // ---- Envs: its tag and pods on each; a row shows that env.
        let mut env_block = div().flex().flex_col().min_w_0().child(table_head(&ENV_COLS, &t));
        for (i, e) in c.envs.iter().enumerate() {
            let here = e.name == self.env;
            let (tag, pods_el): (AnyElement, AnyElement) = match (c.peek(&e.name), c.deployment(&e.name, &self.name)) {
                (_, Some(dep)) => (
                    label(dep.tag()).text_color(if dep.tag() == d.tag() { t.fg } else { t.mute }).into_any_element(),
                    devdeck_ui::pods_bar(&format!("env-pods-{}", e.name), &super::app_view::pod_marks(dep), &t).into_any_element(),
                ),
                (Remote::Loading, None) => (label("loading…").text_color(t.dim).into_any_element(), div().into_any_element()),
                (Remote::Error(_), None) => (label("can't read").text_color(t.warn).into_any_element(), div().into_any_element()),
                (Remote::Ready(_), None) => (label("not deployed").text_color(t.dim).into_any_element(), div().into_any_element()),
            };
            let env_el = row()
                .text_color(if e.protected { t.err } else { t.env })
                .child(label(format!("@{}", e.name)))
                .when(e.protected, |r| r.child(icon(Icon::Protected, t.err, &t).size(small)));
            let (ctx, name, to) = (self.ctx.clone(), self.name.clone(), e.name.clone());
            env_block = env_block.child(
                table_row(("env", i), &ENV_COLS, vec![env_el.into_any_element(), tag, pods_el], &t)
                    .debug_selector({
                        let n = e.name.clone();
                        move || format!("env-{n}")
                    })
                    .when(here, |r| r.bg(t.selection))
                    .when(!here, |r| r.cursor_pointer().hover(|s| s.bg(t.hover)).on_click(move |_, _, cx| ctx.pivot(Activity::Cluster, name.clone(), Some(to.clone()), cx))),
            );
        }
        // A deployment serving apps of its own: its tag is the server's, the same everywhere;
        // each app's tags on the other envs are in its Deploy….
        let envs = d.init_containers.is_empty().then(|| devdeck_ui::panel("svc-envs", "Envs", Vec::new(), env_block, &t));

        // ---- History: what devdeck saw deployed here, newest first.
        let mut hist = div().flex().flex_col().min_w_0();
        if history.is_empty() {
            hist = hist.child(note(label("no deploy seen yet").text_color(t.dim).into_any_element()));
        } else {
            hist = hist.child(table_head(&HISTORY_COLS, &t));
        }
        for (i, r) in history.iter().rev().enumerate() {
            let bad = r.outcome.starts_with("failing") || r.outcome.starts_with("stuck");
            let outcome = if r.outcome.is_empty() { "rolling out".to_string() } else { r.outcome.clone() };
            let what = if r.redeploy() { format!("{} again", r.to) } else { format!("{} → {}", r.from, r.to) };
            let color = if bad { t.err } else if r.outcome.is_empty() { t.accent } else { t.fg };
            hist = hist.child(
                table_row(("history", i), &HISTORY_COLS, vec![
                    label(fmt_age(now.saturating_sub(r.at_s))).text_color(t.mute).into_any_element(),
                    label(r.by.clone()).text_color(t.mute).into_any_element(),
                    label(what).text_color(color).into_any_element(),
                ], &t)
                .tooltip(tip(outcome)),
            );
        }
        let history_panel = devdeck_ui::panel("svc-history", "History", Vec::new(), hist, &t);

        // ---- Containers: what its pods run, from its spec.
        let mut containers = div().flex().flex_col().min_w_0();
        for k in &d.containers {
            let r = k.resources;
            let list = |v: &[String]| if v.is_empty() { "none".to_string() } else { v.join(", ") };
            let ports: Vec<String> = k.ports.iter().map(u16::to_string).collect();
            let value = |text: String| label(text).text_color(t.mute).into_any_element();
            containers = containers.child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .debug_selector({
                        let id = format!("container-{}", k.name);
                        move || id.clone()
                    })
                    .child(div().px_2().pt_1().min_w_0().child(label(k.name.clone()).text_color(t.fg)))
                    .child(devdeck_ui::fields(
                        vec![
                            ("Image".into(), value(k.image.to_string())),
                            ("Ports".into(), value(list(&ports))),
                            ("CPU".into(), value(format!("{} / {}", quantity(r.cpu_request_m, "m"), quantity(r.cpu_limit_m, "m")))),
                            ("Memory".into(), value(format!("{} / {}", quantity(r.mem_request_mi, "Mi"), quantity(r.mem_limit_mi, "Mi")))),
                            ("Config maps".into(), value(list(&k.config_maps))),
                        ],
                        &t,
                    )),
            );
        }
        let containers = devdeck_ui::panel("svc-containers", format!("Containers · {}", d.containers.len()), Vec::new(), containers, &t);

        // A list kept over a lost connection: what's wrong, above it.
        let problem_line = c.problem(&self.env).map(|p| div().flex_none().px_3().pt_2().child(icon_text(Icon::Warning, t.warn, p.to_string(), &t)));
        // Panels on the page's background: what runs on the left (the most room), where
        // and what it ran on the right; one under the other when narrow.
        let left = div().flex_grow(3.).flex_basis(gpui::relative(0.)).min_w(t.size(440.)).flex().flex_col().gap_3().child(deployment).children(parts).child(pods_panel).child(logs);
        let right = div().flex_grow(2.).flex_basis(gpui::relative(0.)).min_w(t.size(300.)).flex().flex_col().gap_3().children(envs).child(history_panel).child(containers);
        let body = div()
            .id("service-body")
            .flex_1()
            .min_h_0()
            .scroll_y()
            .flex()
            .flex_col()
            .children(problem_line)
            .child(div().p_3().flex().flex_wrap().gap_3().child(left).child(right));

        self.window = Some(window.window_handle());
        root.child(body)
    }
}

impl ServiceView {
    /// A pod's row opened: where it runs, how it ended last, its containers (the init ones
    /// first), and its logs.
    fn pod_more(&self, p: &Pod, i: usize, now: u64, me: Entity<Self>, t: &Theme) -> gpui::Div {
        let mut facts: Vec<(SharedString, AnyElement)> = vec![
            ("Node".into(), label(if p.node.is_empty() { "—".to_string() } else { p.node.clone() }).text_color(t.mute).into_any_element()),
            ("Started".into(), label(p.started.map_or_else(|| "not yet".into(), |s| format!("{} ago", fmt_age(now.saturating_sub(s))))).text_color(t.mute).into_any_element()),
            ("Tag".into(), label(p.tag.clone()).text_color(t.mute).into_any_element()),
        ];
        if let Some(l) = &p.last_termination {
            let at = l.at.map(|at| format!(" · {} ago", fmt_age(now.saturating_sub(at)))).unwrap_or_default();
            facts.push(("Last ended".into(), label(format!("{}{at}", l.text())).text_color(t.warn).into_any_element()));
        }
        if let Some(why) = &p.unschedulable {
            facts.push(("Can't be placed".into(), label(why.clone()).text_color(t.err).into_any_element()));
        }
        let mut block = div().flex().flex_col().min_w_0().child(devdeck_ui::fields(facts, t));
        if !p.containers.is_empty() {
            block = block.child(table_head(&CONTAINER_COLS, t));
        }
        let mut containers: Vec<_> = p.containers.iter().collect();
        containers.sort_by_key(|k| !k.init);
        for (j, k) in containers.into_iter().enumerate() {
            let done = k.state.starts_with("terminated: Completed");
            let color = if k.ready || done { t.ok } else if k.state.starts_with("running") { t.warn } else { t.err };
            let name = div()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .child(label(k.name.clone()).text_color(t.fg))
                .when(k.init, |d| d.child(div().flex_none().text_size(t.text_small).text_color(t.dim).child("init")));
            // An init container done: when it ended.
            let state = match (&k.message, k.ended) {
                (_, Some(at)) if done => format!("completed · {} ago", fmt_age(now.saturating_sub(at))),
                (Some(m), _) => format!("{} · {m}", k.state),
                (None, _) => k.state.clone(),
            };
            block = block.child(table_row(("pod-container", i * 64 + j), &CONTAINER_COLS, vec![
                name.into_any_element(),
                label(state).text_color(color).into_any_element(),
                label(if k.restarts > 0 { format!("↻ {}", k.restarts) } else { String::new() }).text_color(t.warn).into_any_element(),
            ], t));
        }
        let pod: SharedString = p.name.clone().into();
        block.child(
            div().px_2().py_1().flex().items_center().gap_1().child(
                devdeck_ui::labeled_button(("pod-logs", i), Icon::Logs, "Logs", t)
                    .debug_selector(move || format!("pod-logs-{i}"))
                    .on_click(move |_, _, cx| me.update(cx, |s, cx| s.logs_of(Some(pod.clone()), cx))),
            ),
        )
    }
}

// ------------------------------------------------------------------ modals

/// The write a confirmation sends. The image shown is what the write is pinned to: it is
/// refused if the cluster changed since.
#[derive(Clone, Debug, PartialEq)]
pub enum WriteKind {
    /// Another branch: the tags `from` → `to`, and the image running when confirming.
    Change { from: String, from_image: String, to: String, part: Option<String> },
    /// The same branch again (`kubectl rollout restart`), what may keep the old build.
    Redeploy { tag: String, image: String, warning: Option<String> },
}

/// "Deploy X on env: from → to" (or "Redeploy X on env?" with its branch) with the
/// equivalent command; a danger modal (R5.6): opens on Cancel, Enter sends from Deploy, Esc
/// cancels. It stays until the cluster answers: closes
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
        #[allow(clippy::disallowed_methods)] // Its Deploy button or Enter on it.
        let ok = danger::confirmed(self);
        let sent = self.service.update(cx, |s, cx| match &kind {
            WriteKind::Change { from_image, to, part: None, .. } => s.deploy(from_image, to, ok, cx),
            WriteKind::Change { from_image, to, part: Some(p), .. } => s.deploy_part(p, from_image, to, ok, cx),
            WriteKind::Redeploy { image, .. } => s.send_redeploy(image, ok, cx),
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

    /// Enter: sends only from Deploy; anywhere else it cancels.
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.0[0].is_focused(window) {
            return self.send(window, cx);
        }
        self.close(window, cx);
    }
}

impl DangerModal for ConfirmDeploy {
    fn cancel(&self) -> &FocusHandle {
        &self.focus.0[1]
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
            .child(div().text_color(t.hi).child(match &self.kind {
                WriteKind::Change { part: Some(p), .. } => format!("Deploy {p} in {} on {}?", self.name, self.env),
                _ => format!("{} {} on {}?", if redeploy { "Redeploy" } else { "Deploy" }, self.name, self.env),
            }))
            // A part: only its init container changes, the deployment's other apps stay.
            .when_some(match &self.kind {
                WriteKind::Change { part: Some(p), .. } => Some(p.clone()),
                _ => None,
            }, |d, p| d.child(label(format!("Only {p}'s init container changes; the other apps keep their tags.")).text_color(t.mute).debug_selector(|| "deploy-part".into())))
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
