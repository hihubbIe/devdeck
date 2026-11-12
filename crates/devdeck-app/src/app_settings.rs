//! App-wide settings state: the stored settings (R9.4, R10.1), the available themes,
//! and which dialog the main window shows (Settings or the notification center, one at a
//! time, as modals). The theme and the UI scale are
//! global. Changing either rebuilds the theme and re-renders every window at once.

use devdeck_core::settings::{DEFAULT_THEME, SettingsStore, scale_down, scale_up};
use devdeck_core::theme_file::{ThemeColors, load_all};
use devdeck_ui::Theme;
use gpui::{App, AppContext, Entity, Global};
use std::path::{Path, PathBuf};

/// The dialog the main window shows over its panes: one at most.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dialog {
    #[default]
    None,
    Settings,
    Notifications,
}

pub struct AppSettings {
    pub store: SettingsStore,
    pub dialog: Dialog,
    /// The view showing it, in the main window's modals (to close it from anywhere).
    pub dialog_view: Option<gpui::EntityId>,
    /// Built-in theme first, then the user's, alphabetical.
    pub themes: Vec<ThemeColors>,
    /// Problems found reading theme files, or with the chosen theme.
    pub theme_problems: Vec<String>,
    pub themes_dir: PathBuf,
}

impl AppSettings {
    /// The chosen theme, or the built-in one if the name isn't known.
    pub fn current_colors(&self) -> &ThemeColors {
        self.themes.iter().find(|t| t.name == self.store.settings.theme).unwrap_or(&self.themes[0])
    }
}

struct AppSettingsHandle(Entity<AppSettings>);
impl Global for AppSettingsHandle {}

/// Loads settings and themes from `dir` and installs the chosen theme.
pub fn init(dir: &Path, cx: &mut App) {
    let store = SettingsStore::load(dir);
    let themes_dir = dir.join("themes");
    let (themes, mut theme_problems) = load_all(&themes_dir);
    if !themes.iter().any(|t| t.name == store.settings.theme) {
        theme_problems.push(format!("Theme `{}` not found, using {DEFAULT_THEME}.", store.settings.theme));
    }
    let settings = AppSettings { store, dialog: Dialog::None, dialog_view: None, themes, theme_problems, themes_dir };
    cx.set_global(Theme::from_colors(settings.current_colors(), settings.store.settings.scale()));
    devdeck_ui::code::sync_theme(cx);
    let entity = cx.new(|_| settings);
    cx.set_global(AppSettingsHandle(entity));
}

pub fn settings(cx: &App) -> Entity<AppSettings> {
    cx.global::<AppSettingsHandle>().0.clone()
}

pub fn scale(cx: &App) -> f32 {
    settings(cx).read(cx).store.settings.scale()
}

fn apply_theme(cx: &mut App) {
    let s = settings(cx).read(cx);
    let theme = Theme::from_colors(s.current_colors(), s.store.settings.scale());
    cx.set_global(theme);
    devdeck_ui::code::sync_theme(cx);
    cx.refresh_windows();
}

/// Sets the UI scale (snapped to a step), saves it, and re-renders every window.
/// A save failure is kept on the store and shown in the settings view.
pub fn set_scale(value: f32, cx: &mut App) {
    settings(cx).update(cx, |s, cx| {
        let _ = s.store.set_scale(value);
        cx.notify();
    });
    apply_theme(cx);
}

/// Picks a theme by name, saves it, and re-renders every window.
pub fn set_theme(name: &str, cx: &mut App) {
    settings(cx).update(cx, |s, cx| {
        if s.themes.iter().any(|t| t.name == name) {
            s.store.settings.theme = name.to_string();
            let _ = s.store.save();
            s.theme_problems.retain(|p| !p.starts_with("Theme `"));
        }
        cx.notify();
    });
    apply_theme(cx);
}

/// Who deploys from this machine, when set (none without settings: tests).
pub fn short_name(cx: &App) -> Option<String> {
    cx.try_global::<AppSettingsHandle>().and_then(|h| h.0.read(cx).store.settings.short_name.clone())
}

/// Checks and saves the short name typed; why not when refused (nothing changes then).
pub fn set_short_name(typed: &str, cx: &mut App) -> Result<(), String> {
    settings(cx).update(cx, |s, cx| {
        let result = s.store.set_short_name(typed);
        cx.notify();
        result
    })
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

/// Whether Settings shows.
pub fn is_open(cx: &App) -> bool {
    dialog(cx) == Dialog::Settings
}

/// Opens Settings (in place of the other dialog) or closes it.
pub fn set_open(open: bool, cx: &mut App) {
    match (open, dialog(cx)) {
        (true, _) => set_dialog(Dialog::Settings, cx),
        (false, Dialog::Settings) => set_dialog(Dialog::None, cx),
        (false, _) => {}
    }
}

/// The dialog the main window shows.
pub fn dialog(cx: &App) -> Dialog {
    settings(cx).read(cx).dialog
}

/// Whether a dialog shows.
pub fn has_dialog(cx: &App) -> bool {
    dialog(cx) != Dialog::None
}

/// Which dialog shows; the main window follows (`WindowView::sync_dialog`).
pub fn set_dialog(dialog: Dialog, cx: &mut App) {
    settings(cx).update(cx, |s, cx| {
        if s.dialog != dialog {
            s.dialog = dialog;
            cx.notify();
        }
    });
}

/// Closes the dialog at once, its modal included, from outside any window (a reveal
/// makes way); the main window then gives the keys where the reveal puts them.
pub fn close_dialog_now(cx: &mut App) {
    let view = settings(cx).update(cx, |s, _| s.dialog_view.take());
    if let Some(id) = view {
        devdeck_ui::overlay::remove_modal(id, cx);
    }
    set_dialog(Dialog::None, cx);
}
