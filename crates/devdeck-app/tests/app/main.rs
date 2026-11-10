//! The app's integration tests, one binary: a new test file is a module here, never a new
//! file in `tests/` (docs/WORKFLOW.md). One area: `cargo nextest run --workspace deck::`.

mod actions;
mod api;
mod api_events;
mod api_reads;
mod api_runs;
mod api_waits;
mod cluster_config;
mod cluster_pods;
mod cluster_views;
mod components;
mod config_workspaces;
mod context_menu;
mod deck;
mod drop_preview;
mod edit_mode;
mod files;
mod git_panel;
mod lifecycle;
mod notes;
mod notes_sidebar;
mod pane_header;
mod ports;
mod pty_driver;
mod runs;
mod service;
mod settings;
mod sidebar;
mod sidebar_groups;
mod tab_labels;
mod terminal;
mod terminals;
mod watcher;
mod workspaces;
