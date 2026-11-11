//! Port (`port/<n>`, R6.9): one page of panels, the Ports full view's details (also a
//! tab). Listening: who holds it (a run, a program in one of our terminals, another
//! process), its pid, how long it has been up, where it listens and whether it answers
//! HTTP, its command line, then its actions together: Open, Copy URL, Restart (a run),
//! Stop holder… (a run at once; another process after a confirmation naming it). Output:
//! the run's or terminal's own output. Wants it: the commands declaring the port or seen
//! on it, their state, Run. Forward: a `kubectl port-forward`'s target, its service in
//! Cluster. While the page is on screen the workspace's scan looks up who holds its port,
//! and an HTTP `HEAD /` asks it every [`PROBE_EVERY`].

use super::ViewCx;
use super::run_view::RunView;
use crate::port_scan::{PortHolder, start_or_toast, stop_holder};
use crate::runner::RunKey;
use crate::services::{PortInfo, PortRow};
use crate::sidebar::Activity;
use devdeck_ui::{Column, Icon, Scrolls, ToastKind, dot, icon_text, label, table_head, table_row, theme, tip, toasts};
use gpui::{
    App, ClipboardItem, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, Window, div, prelude::*,
};
use std::time::Duration;


/// How often a shown page asks its port for an HTTP answer.
pub const PROBE_EVERY: Duration = Duration::from_secs(3);

const WANT_COLS: [Column; 4] = [Column::rest("COMMAND"), Column::fixed("HOW", 90.), Column::fixed("STATE", 90.), Column::fixed("", 56.).right()];

pub struct PortView {
    pub port: String,
    ctx: ViewCx,
    focus: FocusHandle,
    /// Its holder's output, when it's ours: the run or terminal it was made for.
    output: Option<(RunKey, Entity<RunView>)>,
    /// What the last HTTP `HEAD /` got (`200 OK · 14 ms`), while shown.
    pub probe: Option<Result<String, String>>,
    prober: Option<Task<()>>,
    scroll: gpui::ScrollHandle,
    _subs: Vec<Subscription>,
}

impl PortView {
    pub fn new(port: String, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let mut subs = vec![cx.observe(&ctx.services, |_, _, cx| cx.notify())];
        let ports = ctx.services.read(cx).ports.clone();
        subs.push(cx.observe(&ports, |_, _, cx| cx.notify()));
        let me = cx.entity_id();
        let number = port.parse::<u16>().ok();
        let shown = ports.clone();
        subs.extend(ctx.on_visibility(cx, move |this: &mut Self, visible, cx| {
            let Some(n) = number else { return };
            shown.update(cx, |p, cx| if visible { p.show(me, n, cx) } else { p.hide(me) });
            this.prober = visible.then(|| Self::probe_loop(n, cx));
            if !visible {
                this.probe = None;
            }
        }));
        subs.push(cx.on_release(move |_, cx| ports.update(cx, |p, _| p.hide(me))));
        Self { port, ctx, focus: cx.focus_handle(), output: None, probe: None, prober: None, scroll: gpui::ScrollHandle::new(), _subs: subs }
    }

    /// Who holds its port now, if anyone.
    pub fn holder(&self, cx: &App) -> Option<PortInfo> {
        self.row(cx).and_then(|r| r.held)
    }

    /// Its row of the Ports list (the whole machine included).
    pub fn row(&self, cx: &App) -> Option<PortRow> {
        let n = self.port.parse::<u16>().ok()?;
        self.ctx.services.read(cx).port_rows(true, cx).into_iter().find(|r| r.port == n)
    }

    /// Asks the port every [`PROBE_EVERY`], off the UI thread, while a process listens on
    /// it (the demo's holders are made up: nothing to ask).
    fn probe_loop(port: u16, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                let real = this.update(cx, |v, cx| v.row(cx).is_some_and(|r| r.process.is_some())).unwrap_or(false);
                if real {
                    let got = cx.background_spawn(async move { http_probe(port) }).await;
                    if this.update(cx, |v, cx| {
                        v.probe = Some(got);
                        cx.notify();
                    })
                    .is_err()
                    {
                        return;
                    }
                }
                cx.background_executor().timer(PROBE_EVERY).await;
            }
        })
    }

    fn url(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    fn copy_url(&self, cx: &mut App) {
        let url = self.url();
        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
        toasts(cx).update(cx, |t, cx| t.push(ToastKind::Info, format!("Copied {url}."), vec![], cx));
    }

    /// The run or terminal its holder belongs to.
    fn key_of(held: &PortInfo) -> Option<RunKey> {
        held.run.clone().or_else(|| held.terminal.clone())
    }

    /// Its holder's output view, made once for that run or terminal.
    fn output_for(&mut self, key: &RunKey, cx: &mut Context<Self>) -> Entity<RunView> {
        if let Some((k, v)) = &self.output
            && k == key
        {
            return v.clone();
        }
        let (app, cmd, ctx) = (key.app.clone(), key.cmd.clone(), self.ctx.clone());
        let v = cx.new(|cx| RunView::new(app, cmd, ctx, cx));
        self.output = Some((key.clone(), v.clone()));
        v
    }

    fn open_app(&mut self, cx: &mut Context<Self>) {
        if let Some(app) = self.holder(cx).and_then(|h| h.run).map(|k| k.app) {
            self.ctx.pivot(Activity::Apps, app, None, cx);
        }
    }

    fn open_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(n) = self.holder(cx).and_then(|h| h.terminal).and_then(|k| k.terminal_number()) {
            self.ctx.pivot(Activity::Terminals, n.to_string(), None, cx);
        }
    }
}

/// `HEAD /` on `port` of this machine, within a second: the status line and how long it
/// took (`200 OK · 14 ms`), or why not (`no answer`, `not HTTP`).
pub fn http_probe(port: u16) -> Result<String, String> {
    use std::io::{Read, Write};
    let at = std::time::Instant::now();
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|_| "no answer".to_string())?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
    s.write_all(format!("HEAD / HTTP/1.0\r\nHost: localhost:{port}\r\n\r\n").as_bytes()).map_err(|_| "no answer".to_string())?;
    let mut buf = [0u8; 256];
    let n = s.read(&mut buf).map_err(|_| "no answer".to_string())?;
    let status = status_line(&String::from_utf8_lossy(&buf[..n])).ok_or_else(|| "not HTTP".to_string())?;
    Ok(format!("{status} · {} ms", at.elapsed().as_millis()))
}

/// `200 OK` from an HTTP answer's first line (`HTTP/1.1 200 OK`).
pub fn status_line(answer: &str) -> Option<String> {
    let first = answer.lines().next()?;
    let rest = first.strip_prefix("HTTP/")?.split_once(' ')?.1.trim();
    rest.split_whitespace().next()?.parse::<u16>().ok()?;
    Some(rest.to_string())
}

/// Its header offers nothing of its own (R9.11): who holds it links there from its facts.
impl super::Live for PortView {}

impl Focusable for PortView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PortView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let root = div()
            .id("port-view")
            .key_context("PortView")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col();
        let Some(row) = self.row(cx) else {
            return root.child(div().flex_1().flex().items_center().justify_center().text_color(t.mute).child(label(format!("Nothing listens on :{} now.", self.port))));
        };
        let services = self.ctx.services.read(cx);
        let runner = services.runner.read(cx);
        let env = services.env.clone();
        let small = t.icon_small;
        let value = |text: String| label(text).into_any_element();

        // ---- Listening: who, since when, where, and what can be done, together.
        let mut facts: Vec<(SharedString, gpui::AnyElement)> = Vec::new();
        let listening = match &row.held {
            None => {
                facts.push(("State".into(), div().flex().items_center().gap_1().text_color(t.dim).child(dot(t.dim, false, &t)).child("free").into_any_element()));
                devdeck_ui::panel("port-listening", "Listening", Vec::new(), devdeck_ui::fields(facts, &t), &t)
            }
            Some(held) => {
                facts.push(("State".into(), div().flex().items_center().gap_1().text_color(t.ok).child(dot(t.ok, true, &t)).child("listening").into_any_element()));
                // Who holds it, going there: its run's app, or its terminal.
                let go = if held.run.is_some() {
                    Some(devdeck_ui::go_action("port-holder-app", Icon::Apps, "its app in Apps", &t).debug_selector(|| "port-holder-app".into()).on_click(cx.listener(|this, _, _, cx| this.open_app(cx))))
                } else if held.terminal.is_some() {
                    Some(devdeck_ui::go_action("port-holder-terminal", Icon::Terminals, "its terminal in Terminals", &t).debug_selector(|| "port-holder-terminal".into()).on_click(cx.listener(|this, _, _, cx| this.open_terminal(cx))))
                } else {
                    None
                };
                facts.push(("Held by".into(), div().flex().items_center().gap_1().min_w_0().child(label(held.holder.clone())).children(go).into_any_element()));
                if let Some(p) = &row.process {
                    facts.push(("PID".into(), value(p.pid.to_string())));
                }
                let up = match (&held.run, row.process.as_ref().and_then(|p| p.started)) {
                    (Some(key), _) => runner.running_for(key, cx.background_executor().now()).map(super::app_view::lasted),
                    (None, Some(at)) => {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                        Some(super::fmt_age(now.saturating_sub(at)))
                    }
                    _ => None,
                };
                if let Some(up) = up {
                    facts.push(("Up".into(), value(up)));
                }
                if let Some(addr) = row.addr {
                    let (text, color) = if addr.is_unspecified() { (format!("{addr} · every interface"), t.warn) } else { (format!("{addr} · this machine"), t.fg) };
                    facts.push(("Address".into(), label(text).text_color(color).into_any_element()));
                }
                match &self.probe {
                    Some(Ok(got)) => facts.push(("HTTP".into(), label(got.clone()).text_color(if got.starts_with('2') || got.starts_with('3') { t.fg } else { t.warn }).into_any_element())),
                    Some(Err(why)) => facts.push(("HTTP".into(), label(why.clone()).text_color(t.dim).into_any_element())),
                    None => {}
                }
                let command = (!held.command.is_empty()).then(|| div().px_2().pb_1().min_w_0().debug_selector(|| "port-command".into()).child(label(held.command.clone()).text_color(t.mute)));

                // Open, Copy URL, Restart (a run), Stop holder… side by side.
                let url = self.url();
                let holder = match (&held.run, held.pid) {
                    (Some(key), _) => Some(PortHolder::Run { key: key.clone(), title: held.holder.clone() }),
                    (None, Some(pid)) => Some(PortHolder::Process { pid, command: held.command.clone() }),
                    (None, None) => None,
                };
                let stop = {
                    let runner = services.runner.downgrade();
                    let port = row.port;
                    let why = if services.config.is_some() { "another user's process: stop it where it runs" } else { "not started by Kugelblitz: stop it where it runs" };
                    devdeck_ui::labeled_button("stop-holder", Icon::Stop, "Stop holder…", &t)
                        .debug_selector(|| "stop-holder".into())
                        .when(holder.is_none(), |b| b.text_color(t.dim).tooltip(tip(why)))
                        .when_some(holder, move |b, holder| {
                            b.tooltip(tip(format!("stop {holder}"))).on_click(move |_, window, cx| stop_holder(&runner, port, &holder, window, cx))
                        })
                };
                let restart = held.run.clone().map(|key| {
                    let (runner, env) = (services.runner.clone(), env.clone());
                    devdeck_ui::labeled_button("port-restart", Icon::Restart, "Restart", &t).debug_selector(|| "port-restart".into()).on_click(move |_, _, cx| {
                        if let Err(why) = runner.update(cx, |r, cx| r.restart(&key, env.clone(), cx)) {
                            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("Not restarted: {why}."), vec![], cx));
                        }
                    })
                });
                let actions = div()
                    .px_2()
                    .py_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(devdeck_ui::labeled_button("port-open", Icon::OpenOut, "Open", &t).debug_selector(|| "port-open".into()).on_click(move |_, _, cx| cx.open_url(&url)))
                    .child(devdeck_ui::labeled_button("port-copy", Icon::Copy, "Copy URL", &t).debug_selector(|| "port-copy".into()).on_click(cx.listener(|this, _, _, cx| this.copy_url(cx))))
                    .children(restart)
                    .child(stop);
                devdeck_ui::panel("port-listening", "Listening", Vec::new(), div().flex().flex_col().min_w_0().child(devdeck_ui::fields(facts, &t)).children(command).child(actions), &t)
            }
        };

        // ---- Output: the run's or terminal's own, at a fixed height.
        let output = row.held.as_ref().and_then(Self::key_of).map(|key| {
            let v = self.output_for(&key, cx);
            devdeck_ui::panel("port-output", "Output", Vec::new(), div().h(t.size(360.)).min_w_0().flex().flex_col().child(v), &t)
        });
        let services = self.ctx.services.read(cx);
        let runner = services.runner.read(cx);

        // ---- Wants it: what declares the port or was seen on it, and whether it could start.
        let wants = (!row.wants.is_empty()).then(|| {
            let mut block = div().flex().flex_col().min_w_0().child(table_head(&WANT_COLS, &t));
            for (i, w) in row.wants.iter().enumerate() {
                let running = runner.state(&w.key).is_some_and(|s| s.is_running());
                let holds = row.held.as_ref().is_some_and(|h| h.run.as_ref() == Some(&w.key));
                let (state, color) = match (running, row.held.is_some() && !holds) {
                    (true, _) => ("running", t.mute),
                    (false, true) => ("port held", t.warn),
                    (false, false) => ("stopped", t.dim),
                };
                let (r2, key, ctx, app) = (services.runner.clone(), w.key.clone(), self.ctx.clone(), w.key.app.clone());
                let env = env.clone();
                let act = div()
                    .flex()
                    .items_center()
                    .when(!running, |d| {
                        d.child(
                            devdeck_ui::action_button(("want-run", i), Icon::Run, small, true, format!("run {}", runner.title(&w.key)), &t)
                                .on_click(move |_, _, cx| start_or_toast(&r2, &key, env.clone(), cx)),
                        )
                    })
                    .child(devdeck_ui::go_action(("want-app", i), Icon::Apps, format!("{app} in Apps"), &t).on_click(move |_, _, cx| ctx.pivot(Activity::Apps, app.clone(), None, cx)));
                block = block.child(
                    table_row(("want", i), &WANT_COLS, vec![
                        label(runner.title(&w.key)).text_color(t.fg).into_any_element(),
                        label(if w.declared { "declared" } else { "seen on it" }).text_color(t.mute).into_any_element(),
                        label(state).text_color(color).into_any_element(),
                        act.into_any_element(),
                    ], &t)
                    .debug_selector(move || format!("want-{i}"))
                    .when(state == "port held", |r| r.tooltip(tip(format!("a start is refused while :{} is held", row.port)))),
                );
            }
            devdeck_ui::panel("port-wants", format!("Wants it · {}", row.wants.len()), Vec::new(), block, &t)
        });

        // ---- Forward: a `kubectl port-forward`, read from its command line.
        let forward = row.held.as_ref().and_then(|h| devdeck_core::ports::port_forward(&h.command)).map(|f| {
            let mut facts: Vec<(SharedString, gpui::AnyElement)> = Vec::new();
            if let Some(ns) = &f.namespace {
                facts.push(("Env".into(), label(format!("@{ns}")).text_color(t.env).into_any_element()));
            }
            facts.push(("To".into(), value(f.target.clone())));
            facts.push(("Remote port".into(), value(f.remote.to_string())));
            facts.push(("Local".into(), value(format!(":{}", f.local))));
            let go = f.service().map(|svc| {
                let (ctx, svc, ns) = (self.ctx.clone(), svc.to_string(), f.namespace.clone());
                devdeck_ui::go_action("forward-go", Icon::Cluster, format!("{svc} in Cluster"), &t).on_click(move |_, _, cx| ctx.pivot(Activity::Cluster, svc.clone(), ns.clone(), cx)).into_any_element()
            });
            devdeck_ui::panel("port-forward", "Forward", go.into_iter().collect(), devdeck_ui::fields(facts, &t), &t)
        });

        // ---- Not devdeck's: said once.
        let foreign = row.foreign.then(|| {
            let why = if row.held.as_ref().is_some_and(|h| h.pid.is_none()) { "Another user's process: stop it where it runs." } else { "Started outside Kugelblitz: stopped only after a confirmation that names it." };
            devdeck_ui::panel("port-foreign", "Not devdeck's", Vec::new(), div().px_2().py_1().min_w_0().child(icon_text(Icon::Info, t.mute, why, &t)), &t)
        });

        // Panels on the page's background: the holder and its output on the left, the rest
        // on the right; one under the other when narrow.
        let left = div().flex_grow(3.).flex_basis(gpui::relative(0.)).min_w(t.size(420.)).flex().flex_col().gap_3().child(listening).children(output);
        let right = div().flex_grow(2.).flex_basis(gpui::relative(0.)).min_w(t.size(300.)).flex().flex_col().gap_3().children(forward).children(wants).children(foreign);
        let body = div()
            .id("port-body")
            .flex_1()
            .min_h_0()
            .scroll_y()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .child(div().p_3().flex().flex_wrap().gap_3().child(left).child(right));
        root.child(devdeck_ui::with_scrollbar("port-scrollbar", body, &self.scroll).flex_1())
    }
}
