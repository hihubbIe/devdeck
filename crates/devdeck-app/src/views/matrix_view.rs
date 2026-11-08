//! Branch matrix (`matrix`, R7.7): services × envs with the image tag in each cell.
//! Clicking a cell highlights that tag everywhere; Esc clears it. Protected env columns
//! are marked, unreachable envs say so, a service missing from an env shows "—".
//! It watches every env when opened, and those added to the config later. Rows are every deployment found on any env; only the
//! rows on screen are drawn. A cell of a deployment with several containers shows the
//! main one's tag and "+n", the others in its tooltip.

use super::ViewCx;
use crate::cluster::Cluster;
use devdeck_core::cluster::{Deployment, Remote};
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{Scrolls, scroll_list};
use devdeck_ui::{Icon, Theme, icon, label, theme, tip};
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle, Window, actions, div, prelude::*,
};
use std::rc::Rc;

/// Widths at 100 %: the service column and each env's.
const NAME_W: f32 = 220.;
const CELL_W: f32 = 96.;

actions!(matrix_view, [ClearHighlight]);

pub struct MatrixView {
    ctx: ViewCx,
    cluster: Entity<Cluster>,
    focus: FocusHandle,
    pub highlight: Option<String>,
    scroll: UniformListScrollHandle,
    _observe: Subscription,
}

impl MatrixView {
    pub fn new(ctx: ViewCx, cluster: Entity<Cluster>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&cluster, |_, _, cx| cx.notify());
        cluster.update(cx, |c, cx| c.want_all(cx));
        Self { ctx, cluster, focus: cx.focus_handle(), highlight: None, scroll: UniformListScrollHandle::new(), _observe: observe }
    }

    /// The tag in a cell, `None` when the service isn't there.
    pub fn cell(&self, env: &str, service: &str, cx: &App) -> Option<String> {
        self.cluster.read(cx).deployment(env, service).map(Deployment::tag)
    }

    /// One service's row: its name and a cell per env.
    fn row(&self, r: usize, service: &str, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (name_w, cell_w) = (t.size(NAME_W), t.size(CELL_W));
        let c = self.cluster.read(cx);
        let mut row = div().flex().h(t.row_h).items_center().child(label(service.to_string()).flex_none().w(name_w).text_color(t.fg));
        for (k, e) in c.envs.iter().enumerate() {
            let d = c.deployment(&e.name, service);
            let tag = d.map(Deployment::tag);
            let lit = tag.is_some() && tag == self.highlight;
            let hover = t.hover;
            let (ctx, addr) = (self.ctx.clone(), ViewAddr::Svc { env: e.name.clone(), name: service.to_string() });
            let text = match (d, c.peek(&e.name)) {
                (Some(d), _) if d.others() > 0 => format!("{} +{}", d.tag(), d.others()),
                (Some(d), _) => d.tag(),
                (None, Remote::Error(_)) => "?".into(),
                (None, Remote::Loading) => "…".into(),
                (None, Remote::Ready(_)) => "—".into(),
            };
            let others: Vec<String> = d.map(|d| d.containers.iter().filter(|x| x.name != d.main().name).map(|x| format!("{}: {}", x.name, x.image.label())).collect()).unwrap_or_default();
            let tooltip = if others.is_empty() { format!("{text} on {} (double-click opens it)", e.name) } else { format!("{} on {} (double-click opens it); also {}", tag.clone().unwrap_or_default(), e.name, others.join(", ")) };
            let clicked = tag.clone();
            row = row.child(
                div()
                    .id(("cell", r * c.envs.len() + k))
                    .debug_selector({
                        let id = format!("cell-{}-{service}", e.name);
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
                    .tooltip(tip(tooltip))
                    .on_mouse_down(gpui::MouseButton::Right, {
                        let (me, ctx, addr, tag) = (cx.entity(), self.ctx.clone(), addr.clone(), tag.clone());
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
        row.into_any_element()
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
        let services: Rc<Vec<String>> = Rc::new(c.names().into_iter().map(str::to_string).collect());
        let (name_w, cell_w) = (t.size(NAME_W), t.size(CELL_W));
        let mut header = div().flex().h(t.row_h).items_center().child(label("service").debug_selector(|| "matrix-heading".into()).flex_none().w(name_w).text_color(t.dim));
        for (k, e) in c.envs.iter().enumerate() {
            let color = if e.protected { t.err } else { t.env };
            let loading = matches!(c.peek(&e.name), Remote::Loading);
            let problem = c.problem(&e.name).map(str::to_string);
            let cell = div()
                .id(("env", k))
                .flex_none()
                .w(cell_w)
                .min_w_0()
                .flex()
                .items_center()
                .gap_1()
                .text_color(color)
                .when(e.protected, |d| d.child(icon(Icon::Protected, color, &t)))
                .child(label(format!("{}{}", e.name, if loading { " …" } else { "" })))
                .when_some(problem, |d, p| d.child(icon(Icon::Warning, t.warn, &t)).tooltip(tip(p)));
            header = header.child(cell);
        }
        let width = name_w + cell_w * c.envs.len() as f32;
        let rows = services.clone();
        let list = scroll_list(
            "matrix-rows",
            services.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let t = theme(cx).clone();
                range.filter_map(|r| rows.get(r).map(|s| this.row(r, s, &t, cx))).collect()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();
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
            .child(
                div()
                    .id("matrix-scroll")
                    .flex_1()
                    .min_h_0()
                    .scroll_x()
                    .child(div().w(width + t.size(24.)).h_full().px_3().py_1().flex().flex_col().child(header).child(list)),
            )
    }
}
