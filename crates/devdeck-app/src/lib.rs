//! devdeck app library: everything the binaries share (the app itself and the
//! snapshot tool used for visual checks).

pub mod actions;
pub mod app_settings;
pub mod deck;
pub mod kube_feed;
pub mod settings_view;
pub mod stats;
pub mod views;
pub mod window_view;

use devdeck_core::layout::{Drop, Edge, Layout, ViewRef};
use gpui::App;
use std::borrow::Cow;

/// The spike's starting arrangement: logs and a long-titled tab on the left,
/// deployments over notes on the right.
pub fn initial_layout() -> Layout {
    let mut l = Layout::new();
    let a = l.main().focused;
    for v in ["demo/logs", "demo/long", "demo/deployments", "demo/notes"] {
        l.open(ViewRef::new(v), a).expect("open demo view");
    }
    l.move_tab(a, 2, Drop::Edge(a, Edge::Right)).expect("split right");
    let b = l.find_view(&ViewRef::new("demo/deployments")).expect("deployments open").0;
    l.move_tab(a, 2, Drop::Edge(b, Edge::Bottom)).expect("split down");
    l.activate(a, 0).expect("logs active");
    l
}

pub fn load_fonts(cx: &mut App) {
    let fonts = vec![
        Cow::Borrowed(include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMonoNL-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../../assets/fonts/jetbrains-mono/JetBrainsMonoNL-Medium.ttf").as_slice()),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("devdeck: could not load bundled fonts: {e}");
    }
}

/// Fonts, settings (with the theme they select) and key bindings, using the config
/// folder from the environment (`DEVDECK_CONFIG_DIR`, XDG, or `~/.config/devdeck`).
pub fn init(cx: &mut App) {
    init_with_config_dir(&devdeck_core::settings::default_config_dir(), cx);
}

/// Same as [`init`] with an explicit config folder (tests, snapshots).
pub fn init_with_config_dir(dir: &std::path::Path, cx: &mut App) {
    load_fonts(cx);
    app_settings::init(dir, cx);
    actions::bind(cx);
}
