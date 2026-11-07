//! Port (`port/<n>`, R6.9): who holds it and the command, with "Stop holder" when the
//! holder is a devdeck run. Other holders can't be stopped from M1 (the reason says why).

use super::ViewCx;
use crate::services::PortInfo;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{button, label, theme, tip};
use gpui::{Context, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::*};

pub struct PortView {
    pub port: String,
    ctx: ViewCx,
    _observe: Subscription,
}

impl PortView {
    pub fn new(port: String, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&ctx.services, |_, _, cx| cx.notify());
        Self { port, ctx, _observe: observe }
    }

    pub fn holder(&self, cx: &gpui::App) -> Option<PortInfo> {
        let s = self.ctx.services.read(cx);
        s.ports(cx).into_iter().find(|p| p.port.to_string() == self.port)
    }
}

impl Render for PortView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let Some(info) = self.holder(cx) else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.mute)
                .child(label(format!("Nothing listens on :{} now.", self.port)))
                .into_any_element();
        };
        let run = info.run.clone();
        let stop = {
            let (ctx, run) = (self.ctx.clone(), run.clone());
            button("stop-holder", "stop holder", &t)
                .debug_selector(|| "stop-holder".into())
                .when(run.is_none(), |b| b.text_color(t.dim).tooltip(tip("not started by devdeck: stop it where it runs")))
                .when_some(run.zip(Some(info.holder.clone())), move |b, (key, title)| {
                    b.tooltip(tip(format!("stop {title}"))).on_click(move |_, _, cx| {
                        let runner = ctx.services.read(cx).runner.clone();
                        runner.update(cx, |r, cx| r.stop(&key, cx));
                    })
                })
        };
        let open_run = run.map(|key| {
            let ctx = self.ctx.clone();
            button("open-run", "output ›", &t).on_click(move |_, _, cx| ctx.open(ViewAddr::Run { app: key.app.clone(), cmd: key.cmd.clone() }, cx))
        });
        div()
            .size_full()
            .px_3()
            .py_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().flex().items_center().gap_3().child(div().text_color(t.hi).child(format!(":{}", info.port))).child(div().text_color(t.ok).child("● listening")))
            .child(label(format!("held by {}", info.holder)).text_color(t.fg))
            .child(label(info.command.clone()).text_color(t.mute))
            .child(div().flex().gap_2().pt_2().child(stop).children(open_run))
            .into_any_element()
    }
}
