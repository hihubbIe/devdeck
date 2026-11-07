//! Full-window Settings view (R9.4). Replaces the pane area of the main window while
//! open; the title bar stays. Esc or the gear button closes it.

use crate::app_settings::{self, AppSettings};
use devdeck_core::settings::{DEFAULT_THEME, SCALE_STEPS};
use devdeck_ui::{button, kbd, label, section, theme, toggle_button};
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
        let store = app_settings::settings(cx).read(cx).store.clone();
        let scale = store.settings.scale();
        let label_w = t.size(160.);

        let presets = SCALE_STEPS.iter().copied().enumerate().fold(div().flex().gap_1(), |row, (i, step)| {
            row.child(
                toggle_button(("scale-step", i), percent(step), (step - scale).abs() < 1e-4, &t)
                    .on_click(move |_, _, cx| app_settings::set_scale(step, cx)),
            )
        });

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
                    .child(kbd("esc", &t))
                    .child(div().text_color(t.mute).child("close")),
            )
            .child(section("Display", &t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(label_w).flex_none().child("UI scale"))
                    .child(button("scale-down", "−", &t).debug_selector(|| "scale-down".into()).on_click(|_, _, cx| app_settings::decrease(cx)))
                    .child(div().w(t.size(64.)).flex_none().text_color(t.hi).child(percent(scale)))
                    .child(button("scale-up", "+", &t).debug_selector(|| "scale-up".into()).on_click(|_, _, cx| app_settings::increase(cx)))
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
                    .child(kbd("ctrl =", &t))
                    .child(kbd("ctrl -", &t))
                    .child(kbd("ctrl 0", &t))
                    .child(label("bigger, smaller, reset · applied on top of the desktop's own scaling").text_color(t.mute)),
            )
            .child(section("Theme", &t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(label_w).flex_none().child("Theme"))
                    .child(toggle_button("theme-dark", DEFAULT_THEME, store.settings.theme == DEFAULT_THEME, &t))
                    .child(label("more themes come with theme files").text_color(t.mute)),
            )
            .child(section("Files", &t))
            .child(files)
    }
}
