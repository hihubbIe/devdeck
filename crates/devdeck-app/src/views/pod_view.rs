//! Pod (`pod/<env>/<pod>`, R7.2): state, container status, last termination and an
//! events-like timeline derived from what the cluster reports. A pod that went away
//! (rollout, restart) says so.

use super::ViewCx;
use crate::cluster::Cluster;
use devdeck_core::cluster::PodPhase;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{button, label, section, theme};
use gpui::{Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div, prelude::*};

pub struct PodView {
    pub env: String,
    pub pod: String,
    ctx: ViewCx,
    cluster: Entity<Cluster>,
    _observe: Subscription,
}

impl PodView {
    pub fn new(env: String, pod: String, ctx: ViewCx, cluster: Entity<Cluster>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&cluster, |_, _, cx| cx.notify());
        cluster.update(cx, |c, cx| {
            c.deployments(&env, cx);
        });
        Self { env, pod, ctx, cluster, _observe: observe }
    }
}

impl Render for PodView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let c = self.cluster.read(cx);
        let Some((service, p)) = c.find_pod(&self.env, &self.pod).map(|(s, p)| (s.to_string(), p.clone())) else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.mute)
                .child(label(format!("{} is gone from {} (replaced or deleted).", self.pod, self.env)))
                .into_any_element();
        };
        let (phase, ok) = match &p.phase {
            PodPhase::Running => ("Running".to_string(), true),
            PodPhase::Pending => ("Pending".into(), false),
            PodPhase::Waiting(r) => (format!("Waiting: {r}"), false),
            PodPhase::Terminating => ("Terminating".into(), false),
        };
        let mut events = vec![format!("Scheduled on {}", p.node), format!("Pulling image {}", p.tag)];
        match &p.phase {
            PodPhase::Waiting(r) if r == "ImagePullBackOff" => events.push(format!("Failed to pull image tag {}: not found", p.tag)),
            PodPhase::Waiting(r) => {
                events.push("Started container app".into());
                events.push(format!("Back-off restarting failed container ({r}, {} restarts)", p.restarts));
            }
            _ => {
                events.push("Started container app".into());
                if p.ready {
                    events.push("Readiness probe passed".into());
                }
            }
        }
        let ctx = self.ctx.clone();
        let svc = ViewAddr::Svc { env: self.env.clone(), name: service.clone() };
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
                    .child(div().flex_none().text_color(if ok { t.ok } else { t.err }).child(phase))
                    .child(div().flex_1())
                    .child(button("pod-service", format!("{service} ›"), &t).on_click(move |_, _, cx| ctx.open(svc.clone(), cx))),
            )
            .child(section("Status", &t))
            .child(label(format!("ready {} · restarts {} · tag {} · node {}", if p.ready { "yes" } else { "no" }, p.restarts, p.tag, p.node)).text_color(t.mute))
            .child(label(format!("cpu {}m · memory {}Mi", p.usage.cpu_m, p.usage.mem_mi)).text_color(t.mute))
            .children(p.last_termination.clone().map(|l| label(format!("last termination: {l}")).text_color(t.warn)))
            .child(section("Events", &t))
            .children(events.into_iter().map(|e| label(e).text_color(t.mute)))
            .into_any_element()
    }
}
