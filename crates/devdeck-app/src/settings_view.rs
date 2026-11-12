//! Settings (R9.4, `docs/plans/settings-nav.md`): a modal over the main window, opened by
//! its title bar's button. A search on top, the categories on the left (alphabetical),
//! the selected one's page on the right; it opens on the last category shown. Every
//! setting or block is an [`Entry`]: the list, the search and the pages read them, so a
//! new setting is one entry.
//!
//! Keys: opening focuses the search; ↑↓ move through the categories (the matches' while
//! searching), Enter shows the one selected; Tab goes through the search, the list and
//! every control of the page, Enter or space uses the focused control (GPUI's click on a
//! focused element). Esc clears a search, then closes; ctrl+, and a press outside close.

use crate::api::{ApiState, Status};
use crate::app_settings::{self, AppSettings};
use crate::cluster_notes::Notify;
use crate::environment::EnvState;
use devdeck_api::Class;
use devdeck_core::settings::SCALE_STEPS;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::overlay::{ModalClose, ModalDismiss, ModalNext, ModalPrev, kind_look};
use devdeck_ui::{
    Icon, InputEvent, ModalFocus, NavItem, Scrolls, TextInput, Theme, button, focus_ring, icon_button, label, modal_frame, nav_list, section, theme,
    toggle_button, toggle_button_colored, with_scrollbar,
};
use gpui::{
    AnyElement, App, Context, Entity, EntityId, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, actions, div, prelude::*,
};
use std::collections::{HashMap, HashSet};

actions!(settings, [SettingsUp, SettingsDown, SettingsPick]);

/// A page of Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Agents,
    Api,
    Appearance,
    Deploys,
    Files,
    Machine,
    Notifications,
}

impl Category {
    /// In the list's order: alphabetical (R4.1).
    pub const ALL: [Category; 7] = [Category::Agents, Category::Api, Category::Appearance, Category::Deploys, Category::Files, Category::Machine, Category::Notifications];

    pub fn label(self) -> &'static str {
        match self {
            Category::Agents => "Agents",
            Category::Api => "API",
            Category::Appearance => "Appearance",
            Category::Deploys => "Deploys",
            Category::Files => "Files",
            Category::Machine => "Machine",
            Category::Notifications => "Notifications",
        }
    }
}

/// One setting or block of a page: what the search finds, and how it's drawn.
pub struct Entry {
    pub id: &'static str,
    pub category: Category,
    pub label: &'static str,
    /// More words it's found by, the names it shows included.
    pub words: fn(&App) -> String,
    draw: fn(&mut SettingsView, &Theme, &mut Context<SettingsView>) -> AnyElement,
}

/// Every entry, by category then in page order.
pub static ENTRIES: &[Entry] = &[
    Entry { id: "agents", category: Category::Agents, label: "Agents", words: agent_words, draw: draw_agents },
    Entry { id: "api.socket", category: Category::Api, label: "Socket", words: |_| "control api devdeck-ctl mcp".into(), draw: draw_api_socket },
    Entry { id: "api.connected", category: Category::Api, label: "Connected", words: |_| "callers clients connections".into(), draw: draw_api_connected },
    Entry { id: "api.permissions", category: Category::Api, label: "Permissions", words: |_| "policy allow ask terminal outside".into(), draw: draw_api_permissions },
    Entry { id: "api.audit", category: Category::Api, label: "Audit", words: |_| "log history calls".into(), draw: draw_api_audit },
    Entry { id: "appearance.scale", category: Category::Appearance, label: "UI scale", words: |_| "size zoom bigger smaller text".into(), draw: draw_scale },
    Entry { id: "appearance.theme", category: Category::Appearance, label: "Theme", words: theme_words, draw: draw_theme },
    Entry { id: "deploys.short_name", category: Category::Deploys, label: "Short name", words: |_| "who deploys field manager kubectl".into(), draw: draw_short_name },
    Entry { id: "files.folders", category: Category::Files, label: "Folders", words: |_| "config state data settings file machine.toml".into(), draw: draw_folders },
    Entry { id: "machine.node", category: Category::Machine, label: "Node folders", words: |_| "node nvm versions".into(), draw: draw_node },
    Entry { id: "machine.path", category: Category::Machine, label: "PATH additions", words: |_| "path bin".into(), draw: draw_path },
    Entry { id: "machine.shell", category: Category::Machine, label: "Login shell", words: |_| "environment variables problems config".into(), draw: draw_shell },
    Entry { id: "notifications.services", category: Category::Notifications, label: "Services", words: service_words, draw: draw_services },
];

/// Whether every word of `query` starts a word of `text` (case ignored).
pub fn matches(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    let words: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    query.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|q| !q.is_empty()).all(|q| words.iter().any(|w| w.starts_with(q)))
}

pub struct SettingsView {
    /// The page shown (kept: it opens where it was).
    pub category: Category,
    pub search: Entity<TextInput>,
    /// The entry a search led to, tinted until the category or the search changes.
    pub highlight: Option<&'static str>,
    /// While searching: the category Enter shows (`None`: the default one).
    picked: Option<Category>,
    /// The list's keys.
    nav_focus: FocusHandle,
    /// One focus handle per control, by id; `order` is the page's, as last drawn.
    controls: HashMap<SharedString, FocusHandle>,
    order: Vec<FocusHandle>,
    /// The short name as typed.
    pub short_name: Entity<TextInput>,
    /// Why the short name typed was refused.
    pub short_name_error: Option<String>,
    /// The control API's state, followed once it exists (it starts after the window).
    api: Option<(Entity<ApiState>, Subscription)>,
    /// What else it redraws for (the shell, the workspaces' clusters, the environment).
    followed: HashSet<EntityId>,
    follows: Vec<Subscription>,
    /// Its page's scroll, read by its scrollbar.
    scroll: gpui::ScrollHandle,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings: Entity<AppSettings> = app_settings::settings(cx);
        let saved = app_settings::short_name(cx).unwrap_or_default();
        let short_name = cx.new(|cx| {
            let mut i = TextInput::new("f.lastname", cx);
            i.set_text(&saved, cx);
            i
        });
        let search = cx.new(|cx| TextInput::new("search settings", cx));
        let subs = vec![
            cx.observe(&settings, |_, _, cx| cx.notify()),
            cx.subscribe(&short_name, |this: &mut Self, _, e: &InputEvent, cx| match e {
                InputEvent::Submit(_) => this.save_short_name(cx),
                InputEvent::Changed(_) | InputEvent::Cancel => {
                    this.short_name_error = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&search, |this: &mut Self, _, e: &InputEvent, cx| match e {
                // Typing starts a new search; clearing it keeps the tint it led to.
                InputEvent::Changed(text) => {
                    if !text.is_empty() {
                        this.highlight = None;
                    }
                    this.picked = None;
                    cx.notify();
                }
                InputEvent::Submit(_) => this.show_picked(cx),
                InputEvent::Cancel => {}
            }),
        ];
        Self {
            category: Category::Appearance,
            search,
            highlight: None,
            picked: None,
            nav_focus: cx.focus_handle(),
            controls: HashMap::new(),
            order: Vec::new(),
            short_name,
            short_name_error: None,
            api: None,
            followed: HashSet::new(),
            follows: Vec::new(),
            scroll: gpui::ScrollHandle::new(),
            _subs: subs,
        }
    }

    /// Saves the short name typed, or says why not.
    pub fn save_short_name(&mut self, cx: &mut Context<Self>) {
        let typed = self.short_name.read(cx).text().to_string();
        self.short_name_error = app_settings::set_short_name(&typed, cx).err();
        if self.short_name_error.is_none()
            && let Some(saved) = app_settings::short_name(cx)
        {
            self.short_name.update(cx, |i, cx| i.set_text(&saved, cx));
        }
        cx.notify();
    }

    /// Opened: on its last category, no search, nothing tinted.
    pub fn opened(&mut self, cx: &mut Context<Self>) {
        self.highlight = None;
        self.clear_search(cx);
    }

    /// Shows `category`'s page (a search ends).
    pub fn show(&mut self, category: Category, cx: &mut Context<Self>) {
        self.category = category;
        self.highlight = None;
        self.clear_search(cx);
        self.scroll.set_offset(gpui::Point::default());
        cx.notify();
    }

    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.picked = None;
        if !self.search.read(cx).text().is_empty() {
            self.search.update(cx, |i, cx| i.set_text("", cx));
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).text().trim().to_string()
    }

    /// The entries the search finds (none without one).
    pub fn found(&self, cx: &App) -> Vec<&'static Entry> {
        let q = self.query(cx);
        if q.is_empty() {
            return Vec::new();
        }
        ENTRIES.iter().filter(|e| matches(&q, &format!("{} {} {}", e.label, e.category.label(), (e.words)(cx)))).collect()
    }

    /// The list's rows: every category, or while searching those with matches and their
    /// counts.
    pub fn listed(&self, cx: &App) -> Vec<(Category, Option<usize>)> {
        if self.query(cx).is_empty() {
            return Category::ALL.iter().map(|c| (*c, None)).collect();
        }
        let found = self.found(cx);
        Category::ALL.iter().filter_map(|c| Some(found.iter().filter(|e| e.category == *c).count()).filter(|n| *n > 0).map(|n| (*c, Some(n)))).collect()
    }

    /// The category selected in the list: the page's, or while searching the one Enter
    /// shows (the picked one, else the page's if it has matches, else the first).
    pub fn selected(&self, cx: &App) -> Option<Category> {
        if self.query(cx).is_empty() {
            return Some(self.category);
        }
        let listed: Vec<Category> = self.listed(cx).into_iter().map(|(c, _)| c).collect();
        [self.picked, Some(self.category)].into_iter().flatten().find(|c| listed.contains(c)).or(listed.first().copied())
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let listed: Vec<Category> = self.listed(cx).into_iter().map(|(c, _)| c).collect();
        let Some(at) = self.selected(cx).and_then(|c| listed.iter().position(|l| *l == c)) else { return };
        let to = listed[(at as isize + by).clamp(0, listed.len() as isize - 1) as usize];
        if self.query(cx).is_empty() {
            self.show(to, cx);
        } else {
            self.picked = Some(to);
            cx.notify();
        }
    }

    /// Enter while searching: the selected category, its first match tinted.
    fn show_picked(&mut self, cx: &mut Context<Self>) {
        if self.query(cx).is_empty() {
            return;
        }
        let Some(c) = self.selected(cx) else { return };
        let first = self.found(cx).into_iter().find(|e| e.category == c).map(|e| e.id);
        self.show(c, cx);
        self.highlight = first;
    }

    /// A click on a row of the list: its category, the keys in the list.
    fn pick_row(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((c, _)) = self.listed(cx).get(i).copied() else { return };
        if self.query(cx).is_empty() {
            self.show(c, cx);
        } else {
            let first = self.found(cx).into_iter().find(|e| e.category == c).map(|e| e.id);
            self.show(c, cx);
            self.highlight = first;
        }
        self.nav_focus.focus(window, cx);
    }

    /// The focus handle of control `id` (kept across frames), in the page's Tab order.
    fn control(&mut self, id: impl Into<SharedString>, cx: &mut Context<Self>) -> FocusHandle {
        let h = self.controls.entry(id.into()).or_insert_with(|| cx.focus_handle()).clone();
        self.order.push(h.clone());
        h
    }

    /// The search, the list, then the page's controls.
    fn tab_order(&self, cx: &App) -> ModalFocus {
        let mut handles = vec![self.search.read(cx).focus_handle(cx), self.nav_focus.clone()];
        handles.extend(self.order.iter().cloned());
        ModalFocus(handles)
    }

    fn close(cx: &mut App) {
        app_settings::set_open(false, cx);
    }

    /// Redraws when the API, the shell, a workspace's cluster or the environment change.
    fn follow(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = crate::api::state(cx)
            && self.api.as_ref().is_none_or(|(e, _)| *e != s)
        {
            let sub = cx.observe(&s, |_, _, cx| cx.notify());
            self.api = Some((s, sub));
        }
        let env = crate::environment::environment(cx);
        if self.followed.insert(env.entity_id()) {
            self.follows.push(cx.observe(&env, |_, _, cx| cx.notify()));
        }
        let Some(shell) = crate::shell::try_shell(cx) else { return };
        if self.followed.insert(shell.entity_id()) {
            self.follows.push(cx.observe(&shell, |_, _, cx| cx.notify()));
        }
        let clusters: Vec<_> = shell.read(cx).workspaces.iter().filter_map(|w| w.services.read(cx).cluster.clone()).collect();
        for c in clusters {
            if self.followed.insert(c.entity_id()) {
                self.follows.push(cx.observe(&c, |_, _, cx| cx.notify()));
            }
        }
    }

    /// One entry: its caption, then what it draws; tinted when a search led to it.
    fn entry(&mut self, e: &'static Entry, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body = (e.draw)(self, t, cx);
        let id = e.id;
        div()
            .id(id)
            .debug_selector(move || format!("setting-{id}"))
            .flex()
            .min_w_0()
            .overflow_hidden()
            .px_2()
            .py_1()
            .gap_2()
            .rounded(t.radius_s)
            .when(self.highlight == Some(id), |d| d.bg(t.selection))
            .child(div().w(t.size(140.)).flex_none().h(t.button_h).flex().items_center().text_color(t.mute).child(label(e.label)))
            .child(div().flex_1().min_w_0().overflow_hidden().child(body))
            .into_any_element()
    }
}

impl Focusable for SettingsView {
    /// Opening gives the search the keys.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }
}

fn percent(scale: f32) -> String {
    format!("{:.0} %", scale * 100.)
}

/// Audit entries shown (the file has them all).
const AUDIT_SHOWN: usize = 8;

/// `14:02:11`, local time, of milliseconds since the epoch.
fn clock(ms: u64) -> String {
    devdeck_sys::time::local_hms(ms).map(|(h, m, s)| format!("{h:02}:{m:02}:{s:02}")).unwrap_or_default()
}

fn since_ms(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Lines of a block, one under the other.
fn lines(items: impl IntoIterator<Item = AnyElement>) -> AnyElement {
    div().flex().flex_col().gap_1().min_w_0().children(items).into_any_element()
}

/// A dim line.
fn note(text: impl Into<SharedString>, t: &Theme) -> AnyElement {
    label(text.into()).text_color(t.mute).into_any_element()
}

/// A name and its value on one line.
fn pair(name: &str, value: impl Into<SharedString>, t: &Theme) -> AnyElement {
    div().flex().gap_2().min_w_0().child(div().w(t.size(120.)).flex_none().text_color(t.mute).child(label(name.to_string()))).child(label(value.into())).into_any_element()
}

// ------------------------------------------------------------------ Appearance

fn draw_scale(v: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let scale = app_settings::settings(cx).read(cx).store.settings.scale();
    let down = v.control("scale-down", cx);
    let up = v.control("scale-up", cx);
    let mut row = div()
        .flex()
        .items_center()
        .gap_2()
        .overflow_hidden()
        .child(focus_ring(icon_button("scale-down", Icon::Minus, "smaller", t), &down, false, t).debug_selector(|| "scale-down".into()).on_click(|_, _, cx| app_settings::decrease(cx)))
        .child(div().w(t.size(64.)).flex_none().text_color(t.hi).child(percent(scale)))
        .child(focus_ring(icon_button("scale-up", Icon::Plus, "larger", t), &up, false, t).debug_selector(|| "scale-up".into()).on_click(|_, _, cx| app_settings::increase(cx)))
        .child(div().w(t.size(16.)).flex_none());
    for (i, step) in SCALE_STEPS.iter().copied().enumerate() {
        let h = v.control(format!("scale-step-{i}"), cx);
        row = row.child(focus_ring(toggle_button(("scale-step", i), percent(step), (step - scale).abs() < 1e-4, t), &h, true, t).on_click(move |_, _, cx| app_settings::set_scale(step, cx)));
    }
    lines([row.into_any_element(), note("on top of the desktop's own scaling", t)])
}

fn theme_words(cx: &App) -> String {
    let all = app_settings::settings(cx).read(cx);
    format!("colors dark light {}", all.themes.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(" "))
}

fn draw_theme(v: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let all = app_settings::settings(cx).read(cx);
    let current = all.current_colors().name.clone();
    let names: Vec<String> = all.themes.iter().map(|t| t.name.clone()).collect();
    let (problems, dir) = (all.theme_problems.clone(), all.themes_dir.display().to_string());
    let mut row = div().flex().items_center().gap_1().overflow_hidden();
    for (i, name) in names.into_iter().enumerate() {
        let h = v.control(format!("theme-{name}"), cx);
        let selector = format!("theme-{name}");
        row = row.child(
            focus_ring(toggle_button(("theme", i), name.clone(), name == current, t), &h, true, t)
                .debug_selector(move || selector.clone())
                .on_click(move |_, _, cx| app_settings::set_theme(&name, cx)),
        );
    }
    let mut out = vec![row.into_any_element(), note(format!("themes are TOML files in {dir} (copy assets/themes/devdeck-dark.toml)"), t)];
    out.extend(problems.into_iter().map(|p| label(p).text_color(t.warn).into_any_element()));
    lines(out)
}

// ------------------------------------------------------------------ Deploys

fn draw_short_name(v: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let field = v.short_name.read(cx).focus_handle(cx);
    v.order.push(field);
    let save = v.control("save-short-name", cx);
    let row = div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().w(t.size(220.)).flex_none().debug_selector(|| "short-name".into()).child(v.short_name.clone()))
        .child(focus_ring(button("save-short-name", "save", t), &save, false, t).debug_selector(|| "save-short-name".into()).on_click(cx.listener(|this, _, _, cx| this.save_short_name(cx))));
    let saved = app_settings::settings(cx).read(cx).store.settings.short_name.clone();
    let says = match &v.short_name_error {
        Some(why) => label(why.clone()).text_color(t.err).debug_selector(|| "short-name-error".into()).into_any_element(),
        None => note(
            match saved {
                Some(n) => format!("deploys are made by \"{}\"", devdeck_core::cluster::field_manager(&n)),
                None => "who deploys from here: needed before any deploy".to_string(),
            },
            t,
        ),
    };
    lines([row.into_any_element(), says])
}

// ------------------------------------------------------------------ Files

fn draw_folders(_: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let store = app_settings::settings(cx).read(cx).store.clone();
    let config = store.path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let shown = |p: &std::path::Path| devdeck_core::config::shown(p);
    let mut out = vec![
        pair("settings file", shown(&store.path), t),
        pair("config folder", shown(&config), t),
        pair("machine file", shown(&config.join(devdeck_core::config::MACHINE_FILE)), t),
        pair("state folder", shown(&devdeck_core::state::default_state_dir()), t),
        pair("data folder", shown(&devdeck_core::notes::default_data_dir()), t),
    ];
    if let Some(e) = store.load_error {
        out.push(label(format!("Couldn't read the settings, using defaults: {e}")).text_color(t.err).into_any_element());
    }
    if let Some(e) = store.save_error {
        out.push(label(format!("Couldn't save the settings: {e}")).text_color(t.err).into_any_element());
    }
    lines(out)
}

// ------------------------------------------------------------------ API

fn api(v: &SettingsView, cx: &App) -> Option<Entity<ApiState>> {
    v.api.as_ref().map(|(s, _)| s.clone()).or_else(|| crate::api::state(cx))
}

fn draw_api_socket(v: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let Some(api) = api(v, cx) else { return note("not started", t) };
    match &api.read(cx).status {
        Status::On(p) => label(devdeck_core::config::shown(p)).debug_selector(|| "settings-api".into()).into_any_element(),
        Status::Off(why) => label(format!("off: {why}")).text_color(t.warn).debug_selector(|| "settings-api".into()).into_any_element(),
    }
}

fn draw_api_connected(v: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let Some(api) = api(v, cx) else { return note("nobody", t) };
    let api = api.read(cx);
    let mut callers: Vec<(String, &crate::api::Connection)> = api.connections.values().map(|c| (c.who.label(), c)).collect();
    callers.sort_by_key(|c| c.0.to_lowercase());
    if callers.is_empty() {
        return note("nobody", t);
    }
    lines(callers.into_iter().map(|(name, c)| {
        let calls = if c.calls == 1 { "1 call".to_string() } else { format!("{} calls", c.calls) };
        label(format!("{name} · since {} · {calls}", clock(since_ms(c.since)))).into_any_element()
    }))
}

fn draw_api_permissions(_: &mut SettingsView, t: &Theme, _: &mut Context<SettingsView>) -> AnyElement {
    let classes = |p: devdeck_api::Policy| Class::ALL.iter().filter(|c| crate::api::policy::terminal(**c) == p).map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
    lines([
        pair("a terminal may", format!("{} · asks you: {}", classes(devdeck_api::Policy::Allow), classes(devdeck_api::Policy::Ask)), t),
        note("runs: an app's main commands; stops and restarts only what it started", t),
        pair("outside may", format!("only {}", crate::api::policy::OUTSIDE_METHODS.join(", ")), t),
    ])
}

fn draw_api_audit(v: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let Some(api) = api(v, cx) else { return note("kept once the API starts", t) };
    let api = api.read(cx);
    let where_ = api.audit_path.as_ref().map_or_else(|| "kept in memory".to_string(), |p| format!("all of it in {}", p.display()));
    let mut out = vec![note(format!("newest first; {where_}"), t)];
    for e in api.audit.iter().take(AUDIT_SHOWN) {
        let what = [Some(e.event.label().to_string()), Some(e.caller.clone()), e.method.clone(), e.detail.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        out.push(label(format!("{}  {what}", clock(e.at_ms))).into_any_element());
    }
    lines(out)
}

// ------------------------------------------------------------------ Agents and Machine

fn agent_words(cx: &App) -> String {
    let kinds = crate::environment::agent_kinds(cx);
    format!("session command {}", kinds.iter().map(|k| format!("{} {}", k.name, k.id)).collect::<Vec<_>>().join(" "))
}

fn draw_agents(_: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let mut kinds = crate::environment::agent_kinds(cx);
    if kinds.is_empty() {
        return note("no agent: add one to machine.toml ([agents.<id>])", t);
    }
    kinds.sort_by_key(|k| k.name.to_lowercase());
    lines(kinds.into_iter().map(|k| {
        let facts: Vec<&str> = k.fields.keys().map(String::as_str).collect();
        let mut tail = vec![format!("off screen and quiet {} s, it stops", k.idle_stop.as_secs())];
        if !facts.is_empty() {
            tail.push(format!("reports {}", facts.join(", ")));
        }
        if !k.limits.is_empty() {
            tail.push(format!("{} usage limits", k.limits.len()));
        }
        lines([
            div().flex().gap_2().min_w_0().child(label(k.name.clone()).text_color(t.hi)).child(label(k.id.clone()).text_color(t.mute)).into_any_element(),
            pair("new session", k.line("{id}", false), t),
            pair("resumed", k.line("{id}", true), t),
            note(tail.join(" · "), t),
        ])
    }))
}

fn folders(list: &[std::path::PathBuf], none: &'static str, t: &Theme) -> AnyElement {
    if list.is_empty() {
        return note(none, t);
    }
    lines(list.iter().map(|p| label(devdeck_core::config::shown(p)).into_any_element()))
}

fn draw_node(_: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let dirs = crate::environment::environment(cx).read(cx).machine.node_dirs.clone();
    folders(&dirs, "none in machine.toml (node_dirs)", t)
}

fn draw_path(_: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let path = crate::environment::environment(cx).read(cx).machine.path.clone();
    folders(&path, "none in machine.toml (path)", t)
}

fn draw_shell(_: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let env = crate::environment::environment(cx).read(cx);
    let mut out = vec![match (&env.state, env.shell_problems.is_empty()) {
        (EnvState::Loading, _) => note("asking it…", t),
        (EnvState::Ready(_), true) => label("read: runs get its environment").into_any_element(),
        (EnvState::Ready(_), false) => label("not read: runs get devdeck's own environment").text_color(t.warn).into_any_element(),
    }];
    out.extend(env.problems().into_iter().map(|p| label(p).text_color(t.warn).into_any_element()));
    lines(out)
}

// ------------------------------------------------------------------ Notifications

/// A service subscription: its name, env, workspace's id and name, options, cluster.
type Subscribed = (String, String, String, String, std::collections::BTreeSet<Notify>, Entity<crate::cluster::Cluster>);

/// Every service subscription of every workspace, by service then env.
fn subscriptions(cx: &App) -> Vec<Subscribed> {
    let Some(shell) = crate::shell::try_shell(cx) else { return Vec::new() };
    let mut rows = Vec::new();
    for w in &shell.read(cx).workspaces {
        let Some(c) = w.services.read(cx).cluster.clone() else { continue };
        for (env, name, on) in c.read(cx).subscriptions() {
            rows.push((name, env, w.id.clone(), w.name.clone(), on, c.clone()));
        }
    }
    rows.sort_by_key(|r| (r.0.to_lowercase(), r.1.to_lowercase(), r.3.to_lowercase()));
    rows
}

fn service_words(cx: &App) -> String {
    format!("notify subscriptions deploy crash {}", subscriptions(cx).iter().map(|r| format!("{} {}", r.0, r.1)).collect::<Vec<_>>().join(" "))
}

fn draw_services(v: &mut SettingsView, t: &Theme, cx: &mut Context<SettingsView>) -> AnyElement {
    let rows = subscriptions(cx);
    if rows.is_empty() {
        return note("no service notifications: tick an option in a service's notify menu", t);
    }
    let several = crate::shell::try_shell(cx).is_some_and(|s| s.read(cx).workspaces.len() > 1);
    let clusters: Vec<_> = {
        let mut seen = HashSet::new();
        rows.iter().filter(|r| seen.insert(r.5.entity_id())).map(|r| r.5.clone()).collect()
    };
    let mut out = Vec::new();
    for (i, (name, env, ws, ws_name, on, cluster)) in rows.into_iter().enumerate() {
        let title = if several { format!("{name} on {env} · {ws_name}") } else { format!("{name} on {env}") };
        let key = format!("{ws}/{env}/{name}");
        let go = v.control(format!("svc-go-{key}"), cx);
        let view = ViewAddr::Svc { env: env.clone(), name: name.clone() }.to_ref();
        let mut row = div()
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .overflow_hidden()
            .child(div().w(t.size(220.)).flex_none().child(label(title)))
            .child(focus_ring(devdeck_ui::go_action(("svc-go", i), Icon::Cluster, "its page", t), &go, false, t).debug_selector(move || format!("svc-go-{i}")).on_click(move |_, _, cx| {
                if let Some(s) = crate::shell::try_shell(cx) {
                    let _ = s.update(cx, |s, cx| s.reveal(&ws, &view, cx));
                }
            }));
        for what in Notify::ALL {
            let h = v.control(format!("svc-{key}-{}", what.label()), cx);
            let (cluster, env, name) = (cluster.clone(), env.clone(), name.clone());
            let ticked = on.contains(&what);
            row = row.child(
                focus_ring(toggle_button_colored(SharedString::from(format!("svc-{i}-{}", what.label())), what.label(), ticked, kind_look(what.kind(), t).1, t), &h, true, t)
                    .debug_selector(move || format!("svc-{i}-{}", what.label()))
                    .on_click(move |_, _, cx| cluster.update(cx, |c, cx| c.set_notify(&env, &name, what, !ticked, cx))),
            );
        }
        out.push(row.into_any_element());
    }
    let off = v.control("svc-all-off", cx);
    out.push(
        div()
            .child(focus_ring(button("svc-all-off", "Turn all off", t), &off, false, t).debug_selector(|| "svc-all-off".into()).on_click(move |_, _, cx| {
                for c in &clusters {
                    c.update(cx, |c, cx| c.notify_none(cx));
                }
            }))
            .into_any_element(),
    );
    lines(out)
}

// ------------------------------------------------------------------ the modal

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        self.follow(cx);
        self.order.clear();
        let viewport = window.viewport_size();
        // A fixed size whatever shows (R9.10), capped by the window; the list at most a
        // third of it.
        let width = t.size(900.).min(viewport.width - t.size(80.));
        let nav_w = t.size(200.).min(width * 0.3);
        let searching = !self.query(cx).is_empty();
        let listed = self.listed(cx);
        let selected = self.selected(cx);
        let keys = self.nav_focus.is_focused(window) || self.search.read(cx).focus_handle(cx).is_focused(window);
        let items: Vec<NavItem> = listed.iter().map(|(c, n)| n.map_or_else(|| NavItem::new(c.label()), |n| NavItem::new(c.label()).count(n))).collect();
        let me = cx.entity().downgrade();
        let nav = nav_list("settings-nav", &items, selected.and_then(|s| listed.iter().position(|(c, _)| *c == s)), keys, &t, move |i, window, cx| {
            let _ = me.update(cx, |v, cx| v.pick_row(i, window, cx));
        });
        // The page: the category's entries, or every match under its category's heading.
        let shown: Vec<(Category, Vec<&'static Entry>)> = if searching {
            let found = self.found(cx);
            listed.iter().map(|(c, _)| (*c, found.iter().copied().filter(|e| e.category == *c).collect())).collect()
        } else {
            vec![(self.category, ENTRIES.iter().filter(|e| e.category == self.category).collect())]
        };
        let mut page = div().id("settings-page").flex_1().min_h_0().min_w_0().scroll_y().track_scroll(&self.scroll).flex().flex_col();
        if shown.is_empty() {
            page = page.child(div().px_2().pt_2().text_color(t.mute).child("nothing matches"));
        }
        for (c, entries) in shown {
            let me = cx.entity().downgrade();
            let heading = div().id(SharedString::from(format!("settings-heading-{}", c.label()))).child(section(c.label(), &t)).when(searching, |d| {
                d.cursor_pointer().on_click(move |_, window, cx| {
                    let _ = me.update(cx, |v, cx| {
                        let first = v.found(cx).into_iter().find(|e| e.category == c).map(|e| e.id);
                        v.show(c, cx);
                        v.highlight = first;
                        v.nav_focus.focus(window, cx);
                    });
                })
            });
            page = page.child(heading);
            for e in entries {
                page = page.child(self.entry(e, &t, cx));
            }
        }
        let content = div()
            .key_context("Modal Settings")
            .w(width)
            .h(viewport.height * 0.7)
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            // Esc clears a search first; a press outside always closes.
            .on_action(cx.listener(|this, _: &ModalClose, _, cx| {
                if this.query(cx).is_empty() {
                    Self::close(cx);
                } else {
                    this.clear_search(cx);
                    cx.notify();
                }
            }))
            .on_action(|_: &ModalDismiss, _, cx| Self::close(cx))
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.tab_order(cx).cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.tab_order(cx).cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &SettingsUp, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &SettingsDown, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &SettingsPick, _, cx| this.show_picked(cx)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_color(t.hi).child("Settings"))
                    .child(div().flex_1())
                    .child(button("close-settings", "close", &t).on_click(|_, _, cx| Self::close(cx))),
            )
            .child(div().key_context("SettingsSearch").debug_selector(|| "settings-search".into()).child(self.search.clone()))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(div().id("settings-nav-column").key_context("SettingsNav").track_focus(&self.nav_focus).w(nav_w).flex_none().min_w_0().min_h_0().scroll_y().child(nav))
                    .child(with_scrollbar("settings-scrollbar", page, &self.scroll).flex_1().min_w_0()),
            );
        // A control the keys were on and that isn't drawn any more (its row gone, the page
        // changed): the keys go back to the search, so Esc, Tab and a press outside work.
        if let Some(f) = window.focused(cx) {
            let field = self.short_name.read(cx).focus_handle(cx);
            let ours = f == field || self.controls.values().any(|h| *h == f);
            if ours && !self.order.contains(&f) {
                cx.defer_in(window, |this, window, cx| this.search.read(cx).focus_handle(cx).focus(window, cx));
            }
        }
        modal_frame(content.into_any_element(), window, &t)
    }
}
