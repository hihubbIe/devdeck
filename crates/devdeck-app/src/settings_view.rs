//! Full-window Settings view (R9.4). Replaces the pane area of the main window while
//! open; the title bar stays. Esc or the gear button closes it.

use crate::app_settings::{self, AppSettings};
use devdeck_core::settings::SCALE_STEPS;
use devdeck_ui::{Icon, button, icon_button, label, section, theme, toggle_button};
use gpui::{
    Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, prelude::*,
};

pub struct SettingsView {
    focus: FocusHandle,
    _observe: Subscription,
}

impl SettingsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings: Entity<AppSettings> = app_settings::settings(cx);
        let observe = cx.observe(&settings, |_, _, cx| cx.notify());
        Self { focus: cx.focus_handle(), _observe: observe }
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

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
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
            .child(section("Files", &t))
            .child(files)
    }
}
