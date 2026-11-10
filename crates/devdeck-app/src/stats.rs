//! Self-measurement used by `scripts/measure.sh` (R11.4).
//!
//! - `DEVDECK_STATS=<seconds>` prints a stats line on that interval.
//! - `DEVDECK_STRESS=<cycles>` runs the leak run ([`crate::stress`]), then quits.

use std::sync::atomic::{AtomicU64, Ordering};

/// Incremented by every window render; must stay flat while the app is idle.
pub static RENDERS: AtomicU64 = AtomicU64::new(0);

pub fn count_render() {
    RENDERS.fetch_add(1, Ordering::Relaxed);
}

pub fn renders() -> u64 {
    RENDERS.load(Ordering::Relaxed)
}

/// Memory of this process in kB (`devdeck_sys::memory`); zeros where the OS can't say.
pub use devdeck_sys::memory::Memory;

pub fn memory() -> Memory {
    devdeck_sys::memory::footprint().unwrap_or_default()
}
