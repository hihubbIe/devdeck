//! Full-window Settings view (R9.4). Replaces the pane area of the main window while
//! open; the title bar stays. Esc or the gear button closes it. The short name (who
//! deploys from here) is typed and saved with Enter or "save", checked first.

use crate::api::{ApiState, Status};
use crate::app_settings::{self, AppSettings};
use devdeck_api::Class;
use devdeck_core::settings::SCALE_STEPS;
use devdeck_ui::{Icon, InputEvent, TextInput, button, icon_button, label, section, theme, toggle_button};
use gpui::{
    Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, prelude::*,
};

pub struct SettingsView {
    focus: FocusHandle,
    /// The short name as typed.
    pub short_name: Entity<TextInput>,
    /// Why the short name typed was refused.
    pub short_name_error: Option<String>,
    _observe: Subscription,
    _typed: Subscription,
    /// The control API's state, followed once it exists (it starts after the window).
    api: Option<(Entity<ApiState>, Subscription)>,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings: Entity<AppSettings> = app_settings::settings(cx);
        let observe = cx.observe(&settings, |_, _, cx| cx.notify());
        let saved = app_settings::short_name(cx).unwrap_or_default();
        let short_name = cx.new(|cx| {
            let mut i = TextInput::new("f.lastname", cx);
            i.set_text(&saved, cx);
            i
        });
        let typed = cx.subscribe(&short_name, |this: &mut Self, _, e: &InputEvent, cx| match e {
            InputEvent::Submit(_) => this.save_short_name(cx),
            InputEvent::Changed(_) | InputEvent::Cancel => {
                this.short_name_error = None;
                cx.notify();
            }
        });
        Self { focus: cx.focus_handle(), short_name, short_name_error: None, _observe: observe, _typed: typed, api: None }
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
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
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

/// The control API (R13.8): its socket, who is connected, what callers may do, the last
/// audit entries. Read only: permissions change in a later version.
fn api_rows(api: Option<&ApiState>, label_w: gpui::Pixels, t: &devdeck_ui::Theme) -> gpui::Div {
    let row = |name: &'static str, value: gpui::Div| div().flex().min_w_0().child(div().w(label_w).flex_none().text_color(t.mute).child(name)).child(value.flex_1());
    let mut rows = div().flex().flex_col().gap_1().debug_selector(|| "settings-api".into());
    let Some(api) = api else { return rows.child(row("socket", label("not started").text_color(t.mute))) };
    rows = rows.child(match &api.status {
        Status::On(p) => row("socket", label(p.display().to_string())),
        Status::Off(why) => row("socket", label(format!("off: {why}")).text_color(t.warn)),
    });
    let mut callers: Vec<(String, &crate::api::Connection)> = api.connections.values().map(|c| (c.who.label(), c)).collect();
    callers.sort_by_key(|c| c.0.to_lowercase());
    if callers.is_empty() {
        rows = rows.child(row("connected", label("nobody").text_color(t.mute)));
    }
    for (i, (name, c)) in callers.into_iter().enumerate() {
        let calls = if c.calls == 1 { "1 call".to_string() } else { format!("{} calls", c.calls) };
        rows = rows.child(row(if i == 0 { "connected" } else { "" }, label(format!("{name} · since {} · {calls}", clock(since_ms(c.since))))));
    }
    let classes = |p: devdeck_api::Policy| Class::ALL.iter().filter(|c| crate::api::policy::terminal(**c) == p).map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
    rows = rows
        .child(row("a terminal may", label(format!("{} · asks you: {}", classes(devdeck_api::Policy::Allow), classes(devdeck_api::Policy::Ask)))))
        .child(row("", label("runs: an app's main commands; stops and restarts only what it started").text_color(t.mute)))
        .child(row("outside may", label(format!("only {}", crate::api::policy::OUTSIDE_METHODS.join(", ")))));
    let where_ = api.audit_path.as_ref().map_or_else(|| "kept in memory".to_string(), |p| format!("all of it in {}", p.display()));
    rows = rows.child(row("audit", label(format!("newest first; {where_}")).text_color(t.mute)));
    for e in api.audit.iter().take(AUDIT_SHOWN) {
        let what = [Some(e.event.label().to_string()), Some(e.caller.clone()), e.method.clone(), e.detail.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        rows = rows.child(row("", label(format!("{}  {what}", clock(e.at_ms)))));
    }
    rows
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        if let Some(s) = crate::api::state(cx)
            && self.api.as_ref().is_none_or(|(e, _)| *e != s)
        {
            let sub = cx.observe(&s, |_, _, cx| cx.notify());
            self.api = Some((s, sub));
        }
        let all = app_settings::settings(cx).read(cx);
        let store = all.store.clone();
        let current_theme = all.current_colors().name.clone();
        let theme_names: Vec<String> = all.themes.iter().map(|t| t.name.clone()).collect();
        let theme_problems = all.theme_problems.clone();
        let themes_dir = all.themes_dir.display().to_string();
        let scale = store.settings.scale();
        let label_w = t.size(160.);

        let presets = SCALE_STEPS.iter().copied().enumerate().fold(div().flex().gap_1(), |row, (i, step)| {
            row.child(
                toggle_button(("scale-step", i), percent(step), (step - scale).abs() < 1e-4, &t)
                    .on_click(move |_, _, cx| app_settings::set_scale(step, cx)),
            )
        });

        let theme_row = theme_names.into_iter().enumerate().fold(div().flex().gap_1(), |row, (i, name)| {
            let on = name == current_theme;
            row.child(
                toggle_button(("theme", i), name.clone(), on, &t)
                    .debug_selector({
                        let name = name.clone();
                        move || format!("theme-{name}")
                    })
                    .on_click({
                        let name = name.clone();
                        move |_, _, cx| app_settings::set_theme(&name, cx)
                    }),
            )
        });
        let mut theme_notes = div().flex().flex_col().gap_1().mt_1().child(
            div()
                .flex()
                .child(div().w(label_w).flex_none())
                .child(label(format!("add themes as TOML files in {themes_dir} (copy assets/themes/devdeck-dark.toml)")).text_color(t.mute)),
        );
        for p in theme_problems {
            theme_notes = theme_notes.child(div().flex().child(div().w(label_w).flex_none()).child(label(p).text_color(t.warn)));
        }

        let mut files = div().flex().flex_col().gap_1().child(
            div()
                .flex()
                .child(div().w(label_w).flex_none().text_color(t.mute).child("settings file"))
                .child(label(store.path.display().to_string())),
        );
        if let Some(e) = store.load_error.clone() {
            files = files.child(div().text_color(t.err).child(label(format!("Couldn't read the settings, using defaults: {e}"))));
        }
        if let Some(e) = store.save_error.clone() {
            files = files.child(div().text_color(t.err).child(label(format!("Couldn't save the settings: {e}"))));
        }

        div()
            .key_context("Settings")
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .bg(t.bg)
            .px_6()
            .py_4()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_color(t.hi).child("Settings"))
                    .child(div().flex_1())
                    .child(button("close-settings", "close", &t).on_click(|_, window, cx| window.dispatch_action(Box::new(crate::actions::CloseSettings), cx))),
            )
            .child(section("Display", &t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(label_w).flex_none().child("UI scale"))
                    .child(icon_button("scale-down", Icon::Minus, "smaller", &t).debug_selector(|| "scale-down".into()).on_click(|_, _, cx| app_settings::decrease(cx)))
                    .child(div().w(t.size(64.)).flex_none().text_color(t.hi).child(percent(scale)))
                    .child(icon_button("scale-up", Icon::Plus, "larger", &t).debug_selector(|| "scale-up".into()).on_click(|_, _, cx| app_settings::increase(cx)))
                    .child(div().w(t.size(16.)).flex_none())
                    .child(presets),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mt_1()
                    .child(div().w(label_w).flex_none())
                    .child(label("applied on top of the desktop's own scaling").text_color(t.mute)),
            )
            .child(section("Theme", &t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(label_w).flex_none().child("Theme"))
                    .child(theme_row),
            )
            .child(theme_notes)
            .child(section("Deploys", &t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(label_w).flex_none().child("short name"))
                    .child(div().w(t.size(220.)).flex_none().debug_selector(|| "short-name".into()).child(self.short_name.clone()))
                    .child(button("save-short-name", "save", &t).debug_selector(|| "save-short-name".into()).on_click(cx.listener(|this, _, _, cx| this.save_short_name(cx)))),
            )
            .child(
                div().flex().mt_1().child(div().w(label_w).flex_none()).child(match &self.short_name_error {
                    Some(why) => label(why.clone()).text_color(t.err).debug_selector(|| "short-name-error".into()).into_any_element(),
                    None => label(match &store.settings.short_name {
                        Some(n) => format!("deploys are made by \"{}\"", devdeck_core::cluster::field_manager(n)),
                        None => "who deploys from here: needed before any deploy".to_string(),
                    })
                    .text_color(t.mute)
                    .into_any_element(),
                }),
            )
            .child(section("Files", &t))
            .child(files)
            .child(section("API", &t))
            .child(api_rows(self.api.as_ref().map(|(s, _)| s.read(cx)), label_w, &t))
    }
}
