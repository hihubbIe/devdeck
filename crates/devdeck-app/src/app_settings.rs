//! App-wide settings state: the stored settings (R9.4, R10.1) and whether the
//! full-window Settings view is open. Changing the UI scale rebuilds the theme and
//! re-renders every window at once.

use devdeck_core::settings::{SettingsStore, scale_down, scale_up};
use devdeck_ui::Theme;
use gpui::{App, AppContext, Entity, Global};
use std::path::Path;

pub struct AppSettings {
    pub store: SettingsStore,
    pub open: bool,
}

struct AppSettingsHandle(Entity<AppSettings>);
impl Global for AppSettingsHandle {}

/// Loads settings from `dir` and installs the matching theme.
pub fn init(dir: &Path, cx: &mut App) {
    let store = SettingsStore::load(dir);
    cx.set_global(Theme::devdeck_dark(store.settings.scale()));
    let entity = cx.new(|_| AppSettings { store, open: false });
    cx.set_global(AppSettingsHandle(entity));
}

pub fn settings(cx: &App) -> Entity<AppSettings> {
    cx.global::<AppSettingsHandle>().0.clone()
}

pub fn scale(cx: &App) -> f32 {
    settings(cx).read(cx).store.settings.scale()
}

/// Sets the UI scale (snapped to a step), saves it, and re-renders every window.
/// A save failure is kept on the store and shown in the settings view.
pub fn set_scale(value: f32, cx: &mut App) {
    let entity = settings(cx);
    let scale = entity.update(cx, |s, cx| {
        let _ = s.store.set_scale(value);
        cx.notify();
        s.store.settings.scale()
    });
    cx.set_global(Theme::devdeck_dark(scale));
    cx.refresh_windows();
}

pub fn increase(cx: &mut App) {
    set_scale(scale_up(scale(cx)), cx);
}

pub fn decrease(cx: &mut App) {
    set_scale(scale_down(scale(cx)), cx);
}

pub fn reset(cx: &mut App) {
    set_scale(devdeck_core::settings::DEFAULT_SCALE, cx);
}

pub fn is_open(cx: &App) -> bool {
    settings(cx).read(cx).open
}

pub fn set_open(open: bool, cx: &mut App) {
    settings(cx).update(cx, |s, cx| {
        s.open = open;
        cx.notify();
    });
}
