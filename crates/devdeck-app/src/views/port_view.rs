//! Port (`port/<n>`, R6.9): who holds it and the command, with "Stop holder": a devdeck
//! run stops at once, another process after a confirmation naming it. While the view is
//! on screen, the workspace's port scan also looks up who holds its port.

use super::ViewCx;
use crate::port_scan::{PortHolder, stop_holder};
use crate::services::PortInfo;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{button, dot, label, theme, tip};
use gpui::{Context, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::*};

pub struct PortView {
    pub port: String,
    ctx: ViewCx,
    _subs: Vec<Subscription>,
}

impl PortView {
    pub fn new(port: String, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let edit_mode = crate::edit_mode::edit_mode(cx);
        let mut subs = vec![cx.observe(&ctx.services, |_, _, cx| cx.notify()), cx.observe(&edit_mode, |_, _, cx| cx.notify())];
        let ports = ctx.services.read(cx).ports.clone();
        let me = cx.entity_id();
        let number = port.parse::<u16>().ok();
        let shown = ports.clone();
        subs.extend(ctx.on_visibility(cx, move |_, visible, cx| {
            let Some(n) = number else { return };
            shown.update(cx, |p, cx| if visible { p.show(me, n, cx) } else { p.hide(me) });
        }));
        subs.push(cx.on_release(move |_, cx| ports.update(cx, |p, _| p.hide(me))));
        Self { port, ctx, _subs: subs }
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
        let holder = match (&run, info.pid) {
            (Some(key), _) => Some(PortHolder::Run { key: key.clone(), title: info.holder.clone() }),
            (None, Some(pid)) => Some(PortHolder::Process { pid, command: info.command.clone() }),
            (None, None) => None,
        };
        let stop = {
            let runner = self.ctx.services.read(cx).runner.downgrade();
            let port = info.port;
            let why = if self.ctx.services.read(cx).config.is_some() { "another user's process: stop it where it runs" } else { "not started by Kugelblitz: stop it where it runs" };
            button("stop-holder", "stop holder", &t)
                .debug_selector(|| "stop-holder".into())
                .when(holder.is_none(), |b| b.text_color(t.dim).tooltip(tip(why)))
                .when_some(holder, move |b, holder| {
                    // A process devdeck didn't start: only in edit mode (R5.6).
                    let gated = matches!(holder, PortHolder::Process { .. }).then(|| crate::edit_mode::refusal(cx)).flatten();
                    let b = match &gated {
                        Some(why) => b.text_color(t.dim).tooltip(tip(why.clone())),
                        None => b.tooltip(tip(format!("stop {holder}"))),
                    };
                    b.on_click(move |_, window, cx| stop_holder(&runner, port, &holder, window, cx))
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
            .child(div().flex().items_center().gap_3().child(div().text_color(t.hi).child(format!(":{}", info.port))).child(div().flex().items_center().gap_1().text_color(t.ok).child(dot(t.ok, true, &t)).child("listening")))
            .child(label(format!("held by {}", info.holder)).text_color(t.fg))
            .child(label(info.command.clone()).text_color(t.mute))
            .child(div().flex().gap_2().pt_2().child(stop).children(open_run))
            .into_any_element()
    }
}
