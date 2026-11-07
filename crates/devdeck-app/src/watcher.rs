//! Rediscovery (R6.1, R10.1): new projects, changed manifests and config edits reach
//! the workspaces without a restart. Builder R2 of M2 step 3 implements this file; the
//! interface is fixed here.
//!
//! - One `notify` watcher (with `notify-debouncer-full`) for the whole app: every
//!   workspace's `Services::watch` folders, non-recursively, and the config folder and its
//!   `workspaces/` folder. Access events are dropped (reading a file is not a change).
//! - Changes are grouped (about 300 ms). A change under a workspace's folders rescans
//!   that workspace (`Services::rescan(None, …)`); a change to a workspace file reloads the
//!   configs and rescans that workspace with its new config; `machine.toml` reloads the
//!   environment. Adding or removing a workspace file needs a restart: a toast says so.
//! - After a rescan the watched folders change (`Services::watch`): the watcher follows.
//! - The watcher's thread reports to the UI thread over a channel; nothing blocks it.

use crate::shell::Shell;
use gpui::{App, Entity};
use std::path::PathBuf;

/// Keeps the watching alive; dropping it stops it.
pub struct Watcher {
    _private: (),
}

/// Starts watching for the shell's config workspaces and `config_dir`.
pub fn start(shell: &Entity<Shell>, config_dir: PathBuf, cx: &mut App) -> Watcher {
    let _ = (shell, config_dir, cx);
    Watcher { _private: () }
}
