//! Pod (`pod/<env>/<pod>`, R7.2): state, each container's state, restarts and last
//! termination, and the pod's CPU and memory. The cluster's events aren't readable with
//! the user's access, so there are none. A pod that went away (rollout, restart) says so.
//!
//! It watches its deployment's pods while open (the deployment from the pod's name until
//! the pod is known) and samples metrics while visible.

use super::{ViewCx, fmt_age};
use crate::cluster::{Cluster, PodScope, PodsInterest, Sampling};
use devdeck_core::cluster::Remote;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{button, label, section, theme};
use gpui::{Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div, prelude::*};

pub struct PodView {
    pub env: String,
    pub pod: String,
    ctx: ViewCx,
    cluster: Entity<Cluster>,
    /// Its deployment's pods, once the deployment is known, and that deployment.
    pods: Option<(String, PodsInterest)>,
    sampling: Option<Sampling>,
    _subs: Vec<Subscription>,
}

impl PodView {
    pub fn new(env: String, pod: String, ctx: ViewCx, cluster: Entity<Cluster>, cx: &mut Context<Self>) -> Self {
        let mut subs = vec![cx.observe(&cluster, |this, _, cx| {
            this.follow(cx);
            cx.notify();
        })];
        subs.extend(ctx.on_visibility(cx, |this, visible, cx| {
            let env = this.env.clone();
            this.sampling = visible.then(|| this.cluster.update(cx, |c, cx| c.sample(&env, cx)));
        }));
        cluster.update(cx, |c, cx| c.want(&env, cx));
        let mut this = Self { env, pod, ctx, cluster, pods: None, sampling: None, _subs: subs };
        this.follow(cx);
        this
    }

    /// Takes its deployment's pods once the deployment is known (guessed from the pod's
    /// name), and the right ones once the pod's owner says otherwise.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let Some(service) = self.cluster.read(cx).deployment_of_pod(&self.env, &self.pod) else { return };
        if self.pods.as_ref().is_some_and(|(s, _)| *s == service) {
            return;
        }
        let env = self.env.clone();
        let interest = self.cluster.update(cx, |c, cx| c.pods_interest(&env, PodScope::Service(service.clone()), cx));
        self.pods = Some((service, interest));
    }

    pub fn is_sampling(&self) -> bool {
        self.sampling.is_some()
    }
}

impl Render for PodView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let c = self.cluster.read(cx);
        let message = |text: String| div().size_full().flex().items_center().justify_center().text_color(t.mute).child(label(text)).into_any_element();
        let Some((service, p)) = c.find_pod(&self.env, &self.pod).map(|(s, p)| (s.to_string(), p.clone())) else {
            let service = c.deployment_of_pod(&self.env, &self.pod);
            return match service.as_deref().map(|s| c.pods_state(&self.env, s)) {
                None if matches!(c.peek(&self.env), Remote::Loading) => message(format!("loading {}…", self.env)),
                Some(Remote::Loading) => message(format!("loading {}…", self.pod)),
                Some(Remote::Error(e)) => message(e),
                _ => message(format!("{} isn't among {}'s deployments' pods (replaced, deleted, or not a deployment's).", self.pod, self.env)),
            };
        };
        let now = c.now_s();
        let ago = |at: Option<u64>| at.map(|at| format!(" · {} ago", fmt_age(now.saturating_sub(at)))).unwrap_or_default();
        let usage = match (c.usage(&self.env, &p.name), c.metrics_allowed(&self.env)) {
            (_, Err(e)) => e,
            (Some(u), Ok(())) => format!("cpu {}m · memory {}Mi", u.cpu_m, u.mem_mi),
            (None, Ok(())) if c.usage_read(&self.env) => "no cpu or memory reported for it".into(),
            (None, Ok(())) => "cpu and memory: not read yet".into(),
        };
        let ctx = self.ctx.clone();
        let svc = ViewAddr::Svc { env: self.env.clone(), name: service.clone() };
        let mut containers = div().flex().flex_col().gap_1();
        for k in &p.containers {
            let kind = if k.init { "init container" } else { "container" };
            let color = if k.ready || k.state == "running" { t.mute } else { t.warn };
            containers = containers
                .child(label(format!("{kind} {} · {} · {} · restarts {}", k.name, k.state, if k.ready { "ready" } else { "not ready" }, k.restarts)).text_color(color))
                .children(k.last_termination.as_ref().map(|l| label(format!("  last termination: {}{}", l.text(), ago(l.at))).text_color(t.warn)));
        }
        div()
            .size_full()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_none().text_color(t.hi).child(p.name.clone()))
                    .child(div().flex_none().text_color(t.env).child(format!("@{}", self.env)))
                    .child(div().flex_none().text_color(if p.phase.ok() { t.ok } else { t.err }).child(p.phase.text()))
                    .child(div().flex_1())
                    .child(button("pod-service", format!("{service} ›"), &t).on_click(move |_, _, cx| ctx.open(svc.clone(), cx))),
            )
            .child(section("Status", &t))
            .child(
                label(format!(
                    "ready {} · restarts {} · tag {} · node {}{}",
                    if p.ready { "yes" } else { "no" },
                    p.restarts,
                    p.tag,
                    if p.node.is_empty() { "none yet" } else { &p.node },
                    p.started.map(|s| format!(" · started {} ago", fmt_age(now.saturating_sub(s)))).unwrap_or_default()
                ))
                .text_color(t.mute),
            )
            .child(label(usage).text_color(t.mute))
            .children(p.last_termination.clone().map(|l| label(format!("last termination: {}{}", l.text(), ago(l.at))).text_color(t.warn)))
            .child(section("Containers", &t))
            .child(containers)
            .into_any_element()
    }
}

