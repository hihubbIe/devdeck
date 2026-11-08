//! devdeck app library: everything the binaries share (the app itself and the
//! snapshot tool used for visual checks).

pub mod actions;
pub mod app_settings;
pub mod cheat_sheet;
pub mod cluster;
pub mod cluster_pods;
pub mod context_menu;
pub mod deck;
pub mod edit_mode;
pub mod environment;
pub mod fake_cluster;
pub mod fake_driver;
pub mod gallery;
pub mod kube_cluster;
pub mod layouts;
pub mod lifecycle;
pub mod name_prompt;
pub mod palette;
pub mod places;
pub mod port_scan;
pub mod pty_driver;
pub mod registry;
pub mod repo;
pub mod runner;
pub mod send_to;
pub mod services;
pub mod settings_view;
pub mod shell;
pub mod sidebar;
pub mod stats;
pub mod stress;
pub mod views;
pub mod watcher;
pub mod window_view;

use devdeck_core::layout::{Drop, Edge, Layout, ViewRef};
use gpui::App;
use shell::WorkspaceSpec;
use std::borrow::Cow;

/// The spike's starting arrangement: logs and a long-titled tab on the left,
/// the branch matrix over notes on the right.
pub fn initial_layout() -> Layout {
    let mut l = Layout::new();
    let a = l.main().focused;
    for v in ["demo/logs", "demo/long", "matrix", "demo/notes", "demo/gallery"] {
        l.open(ViewRef::new(v), a).expect("open demo view");
    }
    l.move_tab(a, 2, Drop::Edge(a, Edge::Right)).expect("split right");
    let b = l.find_view(&ViewRef::new("matrix")).expect("matrix open").0;
    l.move_tab(a, 2, Drop::Edge(b, Edge::Bottom)).expect("split down");
    l.activate(a, 0).expect("logs active");
    l
}

/// The personal demo workspace: no cluster, just notes and the component gallery.
pub fn personal_layout() -> Layout {
    let mut l = Layout::new();
    let a = l.main().focused;
    for v in ["demo/notes", "demo/gallery"] {
        l.open(ViewRef::new(v), a).expect("open demo view");
    }
    l
}

/// The M1 demo workspaces (M2 reads them from the config).
pub fn demo_workspaces() -> Vec<WorkspaceSpec> {
    vec![WorkspaceSpec::new("personal", "personal", personal_layout), WorkspaceSpec::new("work", "work", initial_layout)]
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
/// Tests: an empty folder that lives as long as the test's app (removed when the app is
/// dropped), so tests never touch real folders nor leave any behind.
pub fn test_dir(tag: &str, cx: &mut App) -> std::path::PathBuf {
    #[derive(Default)]
    struct Kept(Vec<devdeck_core::testing::TempDir>);
    impl gpui::Global for Kept {}
    let dir = devdeck_core::testing::TempDir::new(tag);
    let path = dir.to_path_buf();
    cx.default_global::<Kept>().0.push(dir);
    path
}

pub fn init_with_config_dir(dir: &std::path::Path, cx: &mut App) {
    load_fonts(cx);
    // Before devdeck's keys: it keeps only its components' own (R12).
    devdeck_ui::code::init(cx);
    app_settings::init(dir, cx);
    edit_mode::edit_mode(cx);
    actions::bind(cx);
    cx.on_window_closed(|cx, id| devdeck_ui::overlay::forget_layer(id, cx)).detach();
}
