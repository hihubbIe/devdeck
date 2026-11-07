//! Command palette (R5.3, ctrl+k): one fuzzy search over apps, their commands,
//! deployments of the current env, envs, workspaces, views and actions. `@35` in the
//! query runs a command on that env once; Tab on an app lists its commands (Esc goes
//! back first). Disabled items say why. Lives in the main window.
//!
//! Running an item closes the palette and puts focus back where it was first, so
//! actions dispatch to the right place.

use crate::registry;
use crate::runner::RunKey;
use crate::shell::{self, Shell};
use devdeck_core::apps::Tier;
use devdeck_core::fuzzy;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{InputEvent, TextInput, ToastKind, label, modal_frame, pop_modal, push_modal, theme, toasts};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, Global, InteractiveElement, IntoElement, ParentElement, Render, ScrollStrategy,
    SharedString, StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle, Window, actions, div, prelude::*,
    uniform_list,
};

actions!(palette, [PaletteDown, PaletteUp, PaletteDrill]);

/// What a palette row does.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Action(&'static str),
    App(String),
    Command { app: String, cmd: String },
    Service(String),
    Env(String),
    Workspace(usize),
    Layout(String),
    View(ViewAddr),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub label: String,
    pub detail: String,
    pub item: Item,
    pub disabled: Option<String>,
}

/// This session's picks, most recent first (the empty query shows them first).
#[derive(Default)]
struct Recent(Vec<String>);
impl Global for Recent {}

pub fn open(window: &mut Window, cx: &mut App) {
    let Some(shell) = shell::try_shell(cx) else { return };
    let restore = window.focused(cx);
    let palette = cx.new(|cx| Palette::new(shell, restore, window, cx));
    push_modal(palette.clone().into(), window, cx);
    let f = palette.read(cx).input.read(cx).focus_handle(cx);
    f.focus(window, cx);
}

pub struct Palette {
    shell: Entity<Shell>,
    pub input: Entity<TextInput>,
    /// An app whose commands are listed (Tab).
    pub drill: Option<String>,
    pub selected: usize,
    restore: Option<FocusHandle>,
    scroll: UniformListScrollHandle,
    _sub: Subscription,
}

impl Palette {
    fn new(shell: Entity<Shell>, restore: Option<FocusHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("search apps, commands, services, envs, layouts, actions", cx));
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::Changed(_) => {
                this.selected = 0;
                this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                cx.notify();
            }
            InputEvent::Submit(_) => this.run_selected(window, cx),
            InputEvent::Cancel => this.back(window, cx),
        });
        Self { shell, input, drill: None, selected: 0, restore, scroll: UniformListScrollHandle::new(), _sub: sub }
    }

    /// Splits `@env` out of the query; returns (rest, env).
    fn query(&self, cx: &App) -> (String, Option<String>) {
        let text = self.input.read(cx).text().to_string();
        let mut env = None;
        let mut rest = Vec::new();
        for word in text.split_whitespace() {
            match word.strip_prefix('@') {
                Some(e) if !e.is_empty() => env = Some(e.to_string()),
                _ => rest.push(word),
            }
        }
        let envs: Vec<String> = {
            let s = self.shell.read(cx);
            let services = s.workspaces[s.active].services.read(cx);
            services.catalog.cluster.as_ref().map(|c| c.envs.iter().map(|e| e.name.clone()).collect()).unwrap_or_default()
        };
        // `@35` → dev35: an exact name, else the one env ending with it.
        let env = env.and_then(|e| envs.iter().find(|n| **n == e).or_else(|| envs.iter().find(|n| n.ends_with(&e))).cloned());
        (rest.join(" "), env)
    }

    /// Every candidate, before filtering.
    fn candidates(&self, window: &Window, cx: &App) -> Vec<Entry> {
        let (_, env_override) = self.query(cx);
        let s = self.shell.read(cx);
        let ws = &s.workspaces[s.active];
        let services = ws.services.read(cx);
        let runner = services.runner.read(cx);
        let env = env_override.clone().or(services.env.clone());
        let mut out = Vec::new();
        let command_entry = |app: &str, cmd: &devdeck_core::apps::Command| {
            let key = RunKey::new(app, &cmd.id);
            let running = runner.state(&key).is_some_and(|s| s.is_running());
            Entry {
                label: format!("{app} › {}", cmd.id),
                detail: format!("{}{}{}", if running { "● " } else { "" }, cmd.line, env.as_deref().map(|e| format!(" · @{e}")).unwrap_or_default()),
                item: Item::Command { app: app.into(), cmd: cmd.id.clone() },
                disabled: runner.refusal(&key),
            }
        };
        if let Some(app) = self.drill.as_ref().and_then(|a| services.catalog.app(a)) {
            for tier in Tier::ALL {
                out.extend(app.tier(tier).into_iter().map(|c| command_entry(&app.id, c)));
            }
            return out;
        }
        for app in &services.catalog.apps {
            out.push(Entry {
                label: app.name.clone(),
                detail: format!("app · {}", app.kind),
                item: Item::App(app.id.clone()),
                disabled: runner.tools().unusable(app),
            });
            for c in app.tier(Tier::Primary) {
                out.push(command_entry(&app.id, c));
            }
        }
        if let (Some(cluster), Some(env)) = (services.cluster.as_ref(), services.env.as_ref()) {
            for d in cluster.read(cx).peek(env).ready().cloned().unwrap_or_default() {
                out.push(Entry { label: d.name.clone(), detail: format!("service on {env} · {}", d.tag), item: Item::Service(d.name), disabled: None });
            }
            for e in services.catalog.cluster.iter().flat_map(|c| c.envs.iter()) {
                out.push(Entry {
                    label: format!("@{}", e.name),
                    detail: format!("switch env{}", if e.protected { " · protected" } else { "" }),
                    item: Item::Env(e.name.clone()),
                    disabled: None,
                });
            }
            out.push(Entry { label: "branch matrix".into(), detail: "view".into(), item: Item::View(ViewAddr::Matrix), disabled: None });
        }
        let book = &ws.deck.read(cx).book;
        for name in book.names() {
            let current = name == book.current;
            out.push(Entry {
                label: format!("layout {name}"),
                detail: if current { "current".into() } else { "switch layout".into() },
                item: Item::Layout(name),
                disabled: None,
            });
        }
        for (i, w) in s.workspaces.iter().enumerate() {
            out.push(Entry {
                label: format!("workspace {}", w.name),
                detail: if i == s.active { "current".into() } else { "switch".into() },
                item: Item::Workspace(i),
                disabled: None,
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for spec in registry::all() {
            if matches!(spec.group, "Text" | "Menus" | "Workspaces") || !seen.insert(spec.label) {
                continue;
            }
            let action = (spec.build)();
            let available = self.restore.as_ref().is_some_and(|h| window.is_action_available_in(action.as_ref(), h));
            if !available {
                continue;
            }
            out.push(Entry {
                label: spec.label.to_string(),
                detail: spec.group.to_lowercase(),
                item: Item::Action(spec.id),
                disabled: None,
            });
        }
        out
    }

    /// What the list shows: ranked matches; for an empty query, recent picks first.
    pub fn entries(&self, window: &Window, cx: &App) -> Vec<Entry> {
        let (q, _) = self.query(cx);
        let all = self.candidates(window, cx);
        if q.is_empty() && self.drill.is_none() {
            let recent = cx.try_global::<Recent>().map(|r| r.0.clone()).unwrap_or_default();
            let mut first: Vec<Entry> = recent.iter().filter_map(|l| all.iter().find(|e| &e.label == l).cloned()).collect();
            let mut rest: Vec<Entry> =
                all.into_iter().filter(|e| matches!(e.item, Item::Action(_) | Item::Workspace(_) | Item::Layout(_)) && !recent.contains(&e.label)).collect();
            rest.sort_by_key(|e| e.label.to_lowercase());
            first.extend(rest);
            return first;
        }
        fuzzy::rank(&q, all, |e| e.label.as_str())
    }

    fn move_selection(&mut self, delta: isize, window: &Window, cx: &mut Context<Self>) {
        let n = self.entries(window, cx).len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, n as isize - 1) as usize;
        self.scroll.scroll_to_item(self.selected, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn drill_in(&mut self, window: &Window, cx: &mut Context<Self>) {
        if let Some(Entry { item: Item::App(app), .. }) = self.entries(window, cx).get(self.selected).cloned() {
            self.drill = Some(app);
            self.selected = 0;
            self.input.update(cx, |i, cx| i.set_text("", cx));
            cx.notify();
        }
    }

    /// Esc: out of a drill-down first, then close.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.drill.take().is_some() {
            self.selected = 0;
            self.input.update(cx, |i, cx| i.set_text("", cx));
            cx.notify();
        } else {
            self.close(window, cx);
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        pop_modal(window, cx);
        if let Some(h) = &self.restore {
            h.focus(window, cx);
        }
    }

    pub fn run_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.entries(window, cx);
        let Some(entry) = entries.get(self.selected).cloned() else { return };
        if let Some(why) = &entry.disabled {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("{}: {why}", entry.label), vec![], cx));
            return;
        }
        let (_, env_override) = self.query(cx);
        cx.update_default_global::<Recent, _>(|r, _| {
            r.0.retain(|l| *l != entry.label);
            r.0.insert(0, entry.label.clone());
            r.0.truncate(8);
        });
        self.close(window, cx);
        run(&self.shell, entry.item, env_override, window, cx);
    }
}

/// Runs a palette item (after the palette closed and focus is back).
pub fn run(shell: &Entity<Shell>, item: Item, env_override: Option<String>, window: &mut Window, cx: &mut App) {
    let (deck, services, sidebar) = {
        let s = shell.read(cx);
        let w = &s.workspaces[s.active];
        (w.deck.clone(), w.services.clone(), w.sidebar.clone())
    };
    let open = |addr: ViewAddr, cx: &mut App| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let p = l.main().focused;
                l.open(addr.to_ref(), p).map(|_| ())
            });
        })
    };
    match item {
        Item::Action(id) => {
            if let Some(spec) = registry::by_id(id) {
                window.dispatch_action((spec.build)(), cx);
            }
        }
        Item::App(app) => open(ViewAddr::App { app }, cx),
        Item::View(addr) => open(addr, cx),
        Item::Service(name) => {
            let env = services.read(cx).env.clone().unwrap_or_default();
            open(ViewAddr::Svc { env, name }, cx);
        }
        Item::Env(env) => sidebar.update(cx, |s, cx| s.set_env(&env, cx)),
        Item::Workspace(i) => shell.update(cx, |s, cx| s.switch(i, cx)),
        Item::Layout(name) => {
            if let Err(e) = deck.update(cx, |d, cx| d.switch_layout(&name, cx)) {
                toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, e.to_string(), vec![], cx));
            }
        }
        Item::Command { app, cmd } => {
            let (runner, env) = {
                let s = services.read(cx);
                (s.runner.clone(), env_override.or(s.env.clone()))
            };
            let key = RunKey::new(&app, &cmd);
            if let Err(why) = runner.update(cx, |r, cx| r.start(&key, env, cx)) {
                toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, format!("{}: {why}", key.title()), vec![], cx));
            }
        }
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let entries = self.entries(window, cx);
        let n = entries.len();
        let selected = self.selected.min(n.saturating_sub(1));
        let (_, env) = self.query(cx);
        let header = match (&self.drill, &env) {
            (Some(app), _) => format!("{app} › commands"),
            (None, Some(e)) => format!("runs on @{e}"),
            (None, None) => String::new(),
        };
        let list = uniform_list(
            "palette-rows",
            n,
            cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                let t = theme(cx).clone();
                let rows = this.entries(window, cx);
                range
                    .filter_map(|i| rows.get(i).cloned().map(|e| (i, e)))
                    .map(|(i, e)| {
                        let hover = t.hover;
                        let label_text: SharedString = e.label.clone().into();
                        div()
                            .id(("palette-row", i))
                            .h(t.row_h)
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_3()
                            .rounded(t.radius_s)
                            .when(i == selected, |d| d.bg(t.selected))
                            .hover(move |s| s.bg(hover))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.selected = i;
                                this.run_selected(window, cx);
                            }))
                            .child(div().flex_none().max_w(gpui::relative(0.55)).truncate().text_color(if e.disabled.is_some() { t.dim } else { t.fg }).child(label_text))
                            .child(label(e.disabled.clone().map(|d| format!("⚠ {d}")).unwrap_or(e.detail.clone())).flex_1().text_color(if e.disabled.is_some() { t.warn } else { t.mute }))
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .h(t.size(360.));
        let content = div()
            .key_context("Modal")
            .w(t.size(720.))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .key_context("Palette")
                    .debug_selector(|| "palette-input".into())
                    .on_action(cx.listener(|this, _: &PaletteDown, window, cx| this.move_selection(1, window, cx)))
                    .on_action(cx.listener(|this, _: &PaletteUp, window, cx| this.move_selection(-1, window, cx)))
                    .on_action(cx.listener(|this, _: &PaletteDrill, window, cx| this.drill_in(window, cx)))
                    .child(self.input.clone()),
            )
            .when(!header.is_empty(), |d| d.child(label(header).text_color(t.env)))
            .child(if n == 0 { div().h(t.size(360.)).child(label("no match").text_color(t.dim)).into_any_element() } else { list.into_any_element() });
        modal_frame(content.into_any_element(), window, &t)
    }
}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}
