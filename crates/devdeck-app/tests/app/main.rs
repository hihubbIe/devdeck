//! The app's integration tests, one binary: a new test file is a module here, never a new
//! file in `tests/` (docs/WORKFLOW.md). One area: `cargo nextest run --workspace deck::`.

mod actions;
mod api;
mod api_events;
mod api_git_ports;
mod api_organize;
mod api_present;
mod api_reads;
mod api_runs;
mod api_waits;
mod apps_view;
mod cluster_config;
mod cluster_page;
mod cluster_pods;
mod cluster_views;
mod components;
mod config_workspaces;
mod context_menu;
mod danger;
mod deck;
mod deploys;
mod drop_preview;
mod files;
mod git_panel;
mod lifecycle;
mod logs;
mod notes;
mod notes_sidebar;
mod notifications;
mod pane_header;
mod ports;
mod ports_list;
mod pty_driver;
mod reveal;
mod runs;
mod service;
mod service_notes;
mod settings;
mod sidebar;
mod sidebar_groups;
mod tab_labels;
mod terminal;
mod terminals;
mod watcher;
mod workspaces;

/// A dangerous action's token without its modal (R5.6): the tests set states up directly;
/// the modals have their own tests (`danger`).
#[allow(clippy::disallowed_methods)]
pub fn confirmed() -> devdeck_app::danger::Confirmed {
    devdeck_app::danger::for_tests()
}
