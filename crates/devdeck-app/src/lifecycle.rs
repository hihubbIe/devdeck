//! Lifecycle of real processes (R8.1, R8.2). Builder R3 of M2 step 3 implements this
//! file (and owns the quit code in `shell.rs` and `main.rs`'s window-close fallback
//! during step 3); the interface is fixed here.
//!
//! - One devdeck at a time: a lock in the state folder; a second devdeck says one is
//!   already running and exits (and so never mistakes the first one's runs for
//!   leftovers).
//! - At start, groups a previous devdeck left running (`GroupLog::leftovers`) are offered
//!   in a dialog: "N processes left by a previous devdeck: Kill / Leave".
//! - SIGTERM, SIGHUP and SIGINT to devdeck: signal-hook's thread kills every live group
//!   (`process::kill_all(GroupLog::live())`), then asks the app to quit.
//! - Quitting with runs ("Stop all and quit", and the window-close fallback) stops them,
//!   waits for the longest stop sequence of what runs (`StopKind::total`), then kills
//!   what's left before quitting.

use crate::shell::Shell;
use devdeck_core::process::GroupLog;
use gpui::{App, Entity};
use std::path::Path;

/// Held while this devdeck runs; released when dropped.
pub struct InstanceLock {
    _private: (),
}

/// Claims the state folder for this devdeck, or says another one has it.
pub fn claim(state_dir: &Path) -> Result<InstanceLock, String> {
    let _ = state_dir;
    Ok(InstanceLock { _private: () })
}

/// Signal handling and the leftovers dialog, once the shell is up.
pub fn install(shell: &Entity<Shell>, log: GroupLog, cx: &mut App) {
    let _ = (shell, log, cx);
}
