//! Branch matrix (`matrix`, R7.7): services × envs with the image tag in each cell.
//! Clicking a cell highlights that tag everywhere; Esc clears it. Protected env columns
//! are marked, unreachable envs say so, a service missing from an env shows "—".
//! It loads every env when opened.

use super::ViewCx;
use crate::cluster::Cluster;
use devdeck_core::cluster::Remote;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{Icon, icon, label, theme, tip};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, Subscription, Window, actions, div, prelude::*,
};

actions!(matrix_view, [ClearHighlight]);

pub struct MatrixView {
    ctx: ViewCx,
    cluster: Entity<Cluster>,
    focus: FocusHandle,
    pub highlight: Option<String>,
    _observe: Subscription,
}

impl MatrixView {
    pub fn new(ctx: ViewCx, cluster: Entity<Cluster>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&cluster, |_, _, cx| cx.notify());
        cluster.update(cx, |c, cx| {
            let envs: Vec<String> = c.envs.iter().map(|e| e.name.clone()).collect();
            for e in envs {
                c.deployments(&e, cx);
            }
        });
        Self { ctx, cluster, focus: cx.focus_handle(), highlight: None, _observe: observe }
    }

    /// The tag in a cell, `None` when the service isn't there.
    pub fn cell(&self, env: &str, service: &str, cx: &App) -> Option<String> {
        self.cluster.read(cx).deployment(env, service).map(|d| d.tag.clone())
    }
}

impl Focusable for MatrixView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MatrixView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let c = self.cluster.read(cx);
        let envs = c.envs.clone();
        let mut services = c.spec.services.clone();
        services.sort();
        let (name_w, cell_w) = (t.size(220.), t.size(96.));
        let mut header = div().flex().h(t.row_h).items_center().child(div().flex_none().w(name_w).text_color(t.dim).child("service"));
        for e in &envs {
            let color = if e.protected { t.err } else { t.env };
            let loading = matches!(c.peek(&e.name), Remote::Loading);
            let cell = div()
                .flex_none()
                .w(cell_w)
                .min_w_0()
                .flex()
                .items_center()
                .gap_1()
                .text_color(color)
                .when(e.protected, |d| d.child(icon(Icon::Protected, color, &t)))
                .child(label(format!("{}{}", e.name, if loading { " …" } else { "" })))
                .when(matches!(c.peek(&e.name), Remote::Error(_)), |d| d.child(icon(Icon::Warning, t.warn, &t)));
            header = header.child(cell);
        }
        let mut grid = div().flex().flex_col().child(header);
        for (r, s) in services.iter().enumerate() {
            let mut row = div().flex().h(t.row_h).items_center().child(label(s.clone()).flex_none().w(name_w).text_color(t.fg));
            for (k, e) in envs.iter().enumerate() {
                let tag = c.deployment(&e.name, s).map(|d| d.tag.clone());
                let unreachable = matches!(c.peek(&e.name), Remote::Error(_));
                let lit = tag.is_some() && tag == self.highlight;
                let hover = t.hover;
                let (ctx, addr) = (self.ctx.clone(), ViewAddr::Svc { env: e.name.clone(), name: s.clone() });
                let text = match (&tag, unreachable) {
                    (Some(t), _) => t.clone(),
                    (None, true) => "?".into(),
                    (None, false) => "—".into(),
                };
                let clicked = tag.clone();
                row = row.child(
                    div()
                        .id(("cell", r * 100 + k))
                        .debug_selector({
                            let id = format!("cell-{}-{s}", e.name);
                            move || id.clone()
                        })
                        .flex_none()
                        .w(cell_w)
                        .px_1()
                        .truncate()
                        .rounded(t.radius_s)
                        .text_color(if tag.is_some() { t.mute } else { t.dim })
                        .when(lit, |d| d.bg(t.selection).text_color(t.hi))
                        .when(tag.is_some(), |d| d.hover(move |s| s.bg(hover)).cursor_pointer())
                        .tooltip(tip(format!("{} on {} (double-click opens it)", text, e.name)))
                        .on_mouse_down(gpui::MouseButton::Right, {
                            let (me, ctx, addr, tag) = (cx.entity(), self.ctx.clone(), ViewAddr::Svc { env: e.name.clone(), name: s.clone() }, tag.clone());
                            move |ev, window, cx| {
                                let Some(tag) = tag.clone() else { return };
                                let (ctx, addr, me) = (ctx.clone(), addr.clone(), me.clone());
                                devdeck_ui::open_menu(
                                    ev.position,
                                    vec![
                                        devdeck_ui::MenuItem::new("Open service", move |_, cx| ctx.open(addr.clone(), cx)),
                                        devdeck_ui::MenuItem::new(format!("Highlight {tag}"), move |_, cx| {
                                            me.update(cx, |m, cx| {
                                                m.highlight = Some(tag.clone());
                                                cx.notify();
                                            })
                                        }),
                                    ],
                                    window,
                                    cx,
                                );
                            }
                        })
                        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, _, cx| {
                            if ev.click_count() == 2 {
                                ctx.open(addr.clone(), cx);
                            } else {
                                this.highlight = if this.highlight == clicked { None } else { clicked.clone() };
                                cx.notify();
                            }
                        }))
                        .child(text),
                );
            }
            grid = grid.child(row);
        }
        div()
            .id("matrix-view")
            .key_context("MatrixView")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|this, _: &ClearHighlight, _, cx| {
                this.highlight = None;
                cx.notify();
            }))
            .child(
                div()
                    .flex_none()
                    .h(t.toolbar_h)
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(t.line_soft)
                    .child(div().text_color(t.hi).child("branch matrix"))
                    .child(label(match &self.highlight {
                        Some(h) => format!("highlighting {h}"),
                        None => "click a tag to highlight it everywhere · double-click opens the service".into(),
                    })
                    .text_color(t.mute)),
            )
            .child(div().id("matrix-scroll").flex_1().min_h_0().overflow_scroll().px_3().py_1().child(grid))
    }
}
