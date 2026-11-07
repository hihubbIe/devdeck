//! Service inspector (`svc/<env>/<name>`, R7.2–R7.6, R7.8): header, actions, the image
//! tag editor (suggestions, unsaved marker, ctrl+s → confirmation stating the exact
//! command), rollout progress with one-action Undo, protected-env unlock, then metrics,
//! pods, container and devdeck's deploy history.
//!
//! Metrics are sampled only while the view is visible (it holds a [`Sampling`] handle).
//! Keys (`ServiceView`): ctrl+s save, u undo, l logs, t edit tag; Esc in the tag editor
//! reverts it (the pane then gives the keys back to the view).

use super::ViewCx;
use crate::cluster::{Cluster, Sampling};
use devdeck_core::cluster::{Deployment, Pod, PodPhase, Remote, Rollout, unlock_matches};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{
    InputEvent, MenuItem, ModalFocus, TextInput, Theme, ToastKind, button, label, modal_frame, open_menu, pop_modal, push_modal, section,
    theme, tip, toasts,
};
use gpui::{
    AnyWindowHandle, App, AppContext, Context, Entity, FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, WeakFocusHandle, Window, actions, div, prelude::*, relative,
};

actions!(service_view, [SaveTag, Undo, OpenLogs, EditTag, ConfirmModal]);

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
    /// The window it last drew in: modals open there.
    window: Option<AnyWindowHandle>,
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
        cluster.update(cx, |c, cx| {
            c.deployments(&env, cx);
        });
        let mut this = Self { env, name, ctx, cluster, focus: cx.focus_handle(), tag, shown_tag: None, unlocked: false, sampling: None, window: None, _subs: subs };
        this.cluster_changed(cx);
        this
    }

    pub fn is_sampling(&self) -> bool {
        self.sampling.is_some()
    }

    /// Holds a sampling handle only while on screen.
    fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.sampling = visible.then(|| self.cluster.update(cx, |c, cx| c.sample(cx)));
    }

    /// Keeps the editor on the deployed tag unless the user changed it.
    fn cluster_changed(&mut self, cx: &mut Context<Self>) {
        let current = self.deployment(cx).map(|d| d.tag.clone());
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

    /// Why Save can't go ahead now, if it can't.
    pub fn save_blocked(&self, cx: &App) -> Option<String> {
        let c = self.cluster.read(cx);
        if c.deployment(&self.env, &self.name).is_none() {
            return Some("not deployed here".into());
        }
        if matches!(c.rollout(&self.env, &self.name), Some(Rollout::Progressing { .. })) {
            return Some("a rollout is in progress".into());
        }
        if !self.dirty(cx) {
            return Some("the tag is unchanged".into());
        }
        if self.protected(cx) && !self.unlocked {
            return Some(format!("{} is protected: unlock it first", self.env));
        }
        None
    }

    /// ctrl+s: on a locked protected env asks to unlock first; otherwise confirms.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.protected(cx) && !self.unlocked && self.dirty(cx) {
            self.request_unlock(cx);
            return;
        }
        if let Some(why) = self.save_blocked(cx) {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("Not saved: {why}."), vec![], cx));
            return;
        }
        let to = self.tag.read(cx).text().trim().to_string();
        let (Some(from), Some(command)) = (self.shown_tag.clone(), self.cluster.read(cx).command_for(&self.env, &self.name, &to)) else { return };
        let me = cx.entity();
        let (env, name) = (self.env.clone(), self.name.clone());
        self.in_window(cx, move |window, cx| {
            let modal = cx.new(|cx| ConfirmDeploy::new(me, env, name, from, to, command, window, cx));
            show_modal(modal, window, cx);
        });
    }

    /// Runs the confirmed change.
    pub fn deploy(&mut self, cx: &mut Context<Self>) {
        let to = self.tag.read(cx).text().trim().to_string();
        let (env, name, unlocked) = (self.env.clone(), self.name.clone(), self.unlocked);
        let result = self.cluster.update(cx, |c, cx| c.set_image(&env, &name, &to, unlocked, cx));
        if let Err(e) = result {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Err, format!("{env} › {name}: {e}"), vec![], cx));
        }
        cx.notify();
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if self.protected(cx) && !self.unlocked {
            self.request_unlock(cx);
            return;
        }
        let (env, name, unlocked) = (self.env.clone(), self.name.clone(), self.unlocked);
        if let Err(e) = self.cluster.update(cx, |c, cx| c.undo(&env, &name, unlocked, cx)) {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("{env} › {name}: {e}"), vec![], cx));
        }
        self.revert(cx);
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

    fn open_logs(&mut self, cx: &mut Context<Self>) {
        self.ctx.open(ViewAddr::Logs { env: self.env.clone(), name: self.name.clone() }, cx);
    }

    /// The tag suggestions menu (snapshots open it directly).
    pub fn suggestions_for(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        Self::suggestions(this, cx)
    }

    fn suggestions(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let s = this.read(cx);
        let list = s.cluster.read(cx).tag_suggestions(&s.env, &s.name);
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

fn fmt_age(s: u64) -> String {
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

/// A usage bar against request (tick) and limit (full width).
fn bar(used: u32, request: u32, limit: u32, t: &Theme) -> impl IntoElement {
    let limit = limit.max(1) as f32;
    let fill = (used as f32 / limit).min(1.0);
    let color: Hsla = if used > request { t.warn } else { t.ok };
    div()
        .relative()
        .w(t.size(220.))
        .h(t.size(8.))
        .rounded(t.radius_s)
        .bg(t.line)
        .child(div().absolute().left_0().top_0().h_full().w(relative(fill)).rounded(t.radius_s).bg(color))
        .child(div().absolute().top_0().h_full().w(t.hairline).left(relative((request as f32 / limit).min(1.0))).bg(t.fg))
}

fn phase_text(p: &Pod) -> (String, bool) {
    match &p.phase {
        PodPhase::Running => ("Running".into(), true),
        PodPhase::Pending => ("Pending".into(), false),
        PodPhase::Waiting(r) => (r.clone(), false),
        PodPhase::Terminating => ("Terminating".into(), false),
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
            .on_action(cx.listener(|this, _: &SaveTag, _, cx| this.save(cx)))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &OpenLogs, _, cx| this.open_logs(cx)))
            .on_action(cx.listener(|this, _: &EditTag, window, cx| {
                this.tag.read(cx).focus_handle(cx).focus(window, cx);
            }));
        let message = |text: String, color: Hsla| div().flex_1().flex().items_center().justify_center().text_color(color).child(label(text));
        let d = match (&state, c.deployment(&self.env, &self.name)) {
            (Remote::Loading, _) => return root.child(message(format!("loading {}…", self.env), t.mute)),
            (Remote::Error(e), _) => return root.child(message(format!("⚠ {e}"), t.warn)),
            (Remote::Ready(_), None) => return root.child(message(format!("{} isn't deployed in {}.", self.name, self.env), t.mute)),
            (Remote::Ready(_), Some(d)) => d.clone(),
        };
        let rollout = c.rollout(&self.env, &self.name).cloned();
        let pods: Vec<Pod> = c.pods(&self.env, &self.name).to_vec();
        let history = c.history(&self.env, &self.name).to_vec();
        let metrics = c.metrics_allowed(&self.env);
        let k9s_missing = !self.ctx.services.read(cx).catalog.tools.found.contains("k9s");
        let dirty = self.dirty(cx);
        let blocked = self.save_blocked(cx);
        let (status, status_color) = match &rollout {
            Some(Rollout::Progressing { updated, of, .. }) => (format!("◐ rolling {updated}/{of}"), t.accent),
            Some(Rollout::Failed { .. }) => ("✖ rollout failed".to_string(), t.err),
            _ if d.ready == d.replicas => (format!("● ready {}/{}", d.ready, d.replicas), t.ok),
            _ => (format!("⚠ ready {}/{}", d.ready, d.replicas), t.warn),
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
            .child(div().flex_none().text_color(if protected { t.err } else { t.env }).child(if protected { format!("⊘ @{}", self.env) } else { format!("@{}", self.env) }))
            .when(protected, |h| {
                h.child(div().flex_none().text_color(if self.unlocked { t.warn } else { t.mute }).child(if self.unlocked { "unlocked" } else { "locked" }))
            })
            .child(div().flex_none().text_color(status_color).child(status))
            .child(label(format!("{} · since {}", d.tag, fmt_age(d.since_s))).text_color(t.mute).flex_1());

        let action = |id: &'static str, text: &'static str, why: Option<String>| {
            let enabled = why.is_none();
            button(id, text, &t).when(!enabled, |b| b.text_color(t.dim)).tooltip(tip(why.unwrap_or_else(|| text.to_string())))
        };
        let later = || Some("comes with the real cluster (M3)".to_string());
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
            .child(action("restart", "restart", later()))
            .child(action("scale", "replicas", later()))
            .child(action("port-forward", "port-forward", later()))
            .child(action("shell", "shell", later()))
            .child(action("k9s", "k9s", if k9s_missing { Some("k9s not found".into()) } else { later() }))
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
                button("suggest", "▾", &t).tooltip(tip("tags on other envs and history")).on_click(move |e, window, cx| {
                    let items = ServiceView::suggestions(&me, cx);
                    open_menu(e.position(), items, window, cx);
                })
            })
            .child(div().flex_none().text_color(if dirty { t.warn } else { t.bg }).child("● unsaved"))
            .child(
                button("save", "save", &t)
                    .debug_selector(|| "tag-save".into())
                    .when(blocked.is_some(), |b| b.text_color(t.dim))
                    .tooltip(tip(blocked.clone().unwrap_or_else(|| "confirm and deploy".into())))
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .child(label(d.repo.clone()).text_color(t.dim).flex_1());

        let rollout_line = rollout.map(|r| {
            let (text, color, undo) = match &r {
                Rollout::Progressing { from, to, updated, of } => (format!("◐ rolling out {from} → {to}: {updated}/{of} pods updated"), t.accent, false),
                Rollout::Failed { to, reason, .. } => (format!("✖ rollout to {to} failed: {reason}"), t.err, true),
                Rollout::Done { from, to } => (format!("✓ now running {to} (was {from})"), t.ok, true),
            };
            div()
                .flex_none()
                .px_3()
                .pb_2()
                .flex()
                .items_center()
                .gap_2()
                .child(label(text).text_color(color).flex_1())
                .when(undo, |d| {
                    d.child(button("undo", "undo", &t).debug_selector(|| "rollout-undo".into()).on_click(cx.listener(|this, _, _, cx| this.undo(cx))))
                })
        });

        let res = d.resources;
        let (cpu, mem) = pods.iter().filter(|p| p.ready).fold((0u32, 0u32), |(c, m), p| (c + p.usage.cpu_m, m + p.usage.mem_mi));
        let n = d.replicas.max(1);
        let metrics_el = match metrics {
            Err(e) => div().child(label(format!("⚠ {e}")).text_color(t.warn)),
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
                        .child(bar(cpu, res.cpu_request_m * n, res.cpu_limit_m * n, &t))
                        .child(label(format!("{cpu}m · request {}m · limit {}m", res.cpu_request_m * n, res.cpu_limit_m * n)).text_color(t.mute)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(div().w(t.size(40.)).text_color(t.mute).child("mem"))
                        .child(bar(mem, res.mem_request_mi * n, res.mem_limit_mi * n, &t))
                        .child(label(format!("{mem}Mi · request {}Mi · limit {}Mi", res.mem_request_mi * n, res.mem_limit_mi * n)).text_color(t.mute)),
                ),
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
        if pods.is_empty() {
            pod_rows = pod_rows.child(label("no pods").text_color(t.dim));
        }
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
                            open_menu(
                                e.position,
                                vec![
                                    MenuItem::new("Open pod", move |_, cx| c1.open(pod.clone(), cx)),
                                    MenuItem::new("Logs", move |_, cx| c2.open(logs.clone(), cx)),
                                ],
                                window,
                                cx,
                            );
                        }
                    })
                    .on_click(move |_, _, cx| ctx.open(addr.clone(), cx))
                    .child(col(300., p.name.clone(), t.fg))
                    .child(col(150., phase, if ok { t.ok } else { t.err }))
                    .child(col(60., if p.ready { "✓".into() } else { "✗".into() }, if p.ready { t.ok } else { t.err }))
                    .child(col(70., p.restarts.to_string(), if p.restarts > 3 { t.warn } else { t.mute }))
                    .child(col(50., fmt_age(p.age_s), t.mute))
                    .child(col(60., format!("{}m", p.usage.cpu_m), t.mute))
                    .child(col(60., format!("{}Mi", p.usage.mem_mi), t.mute))
                    .child(col(100., p.tag.clone(), t.mute))
                    .child(label(format!("{}{}", p.node, p.last_termination.as_ref().map(|l| format!(" · {l}")).unwrap_or_default())).flex_1().text_color(if p.last_termination.is_some() { t.warn } else { t.dim })),
            );
        }

        let container = div()
            .flex()
            .flex_col()
            .child(label(format!("container {} · image {}", d.container, d.image())).text_color(t.fg))
            .child(label(format!("ports {}", d.ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "))).text_color(t.mute))
            .child(label(format!("request {}m / {}Mi · limit {}m / {}Mi", res.cpu_request_m, res.mem_request_mi, res.cpu_limit_m, res.mem_limit_mi)).text_color(t.mute))
            .child(label(format!("configmap {}", d.configmap.clone().unwrap_or_else(|| "none".into()))).text_color(t.mute));

        let mut hist = div().flex().flex_col();
        if history.is_empty() {
            hist = hist.child(label("no deploys from devdeck yet").text_color(t.dim));
        }
        for r in history.iter().rev() {
            let failed = r.outcome.starts_with("failed");
            hist = hist.child(label(format!("{} → {} · {}", r.from, r.to, r.outcome)).text_color(if failed { t.err } else { t.mute }));
        }

        let body = div()
            .id("service-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3()
            .pb_3()
            .flex()
            .flex_col()
            .gap_1()
            .child(section("Metrics", &t))
            .child(metrics_el)
            .child(section("Pods", &t))
            // Wider than a narrow pane: scrolls sideways instead of clipping (R3.9).
            .child(div().id("pods-scroll").w_full().overflow_x_scroll().child(pod_rows.min_w(t.size(1100.))))
            .child(section("Container", &t))
            .child(container)
            .child(section("Deploy history", &t))
            .child(hist);

        self.window = Some(window.window_handle());
        root.child(header).child(actions).child(editor).children(rollout_line).child(body)
    }
}

// ------------------------------------------------------------------ modals

/// "Deploy X on env: from → to" with the exact command; Enter deploys, Esc cancels.
pub struct ConfirmDeploy {
    service: Entity<ServiceView>,
    env: String,
    name: String,
    from: String,
    to: String,
    command: String,
    focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
}

impl ConfirmDeploy {
    #[allow(clippy::too_many_arguments)]
    fn new(service: Entity<ServiceView>, env: String, name: String, from: String, to: String, command: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let restore = window.focused(cx).map(|f| f.downgrade());
        Self { service, env, name, from, to, command, focus: ModalFocus(vec![cx.focus_handle(), cx.focus_handle()]), restore }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.0[1].is_focused(window) {
            return self.close(window, cx);
        }
        self.close(window, cx);
        self.service.update(cx, |s, cx| s.deploy(cx));
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
            .child(div().text_color(t.hi).child(format!("Deploy {} on {}?", self.name, self.env)))
            .child(div().flex().gap_2().child(label(self.from.clone()).text_color(t.mute)).child("→").child(label(self.to.clone()).text_color(t.hi)))
            .child(label("Runs:").text_color(t.mute))
            // The one place text wraps (R4.4 allows a view to opt in): the command is
            // shown whole, so the tag being deployed is always visible.
            .child(div().px_2().py_1().rounded(t.radius_s).bg(t.bg).text_color(t.fg).debug_selector(|| "deploy-command".into()).child(self.command.clone()))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(focus_button("deploy-cancel", "Cancel", &self.focus.0[1], &t).on_click(cx.listener(|this, _, w, cx| this.close(w, cx))))
                    .child(
                        focus_button("deploy-confirm", "Deploy", &self.focus.0[0], &t)
                            .debug_selector(|| "deploy-confirm".into())
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.close(w, cx);
                                this.service.update(cx, |s, cx| s.deploy(cx));
                            })),
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
            .child(div().text_color(t.err).child(format!("⊘ {} is protected", self.env)))
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
