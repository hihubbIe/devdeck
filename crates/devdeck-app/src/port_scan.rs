//! Ports of real runs (R6.9). Builder R4 of M2 step 3 implements this file (and owns the
//! port parts of `services.rs`, `runner.rs`'s start and `views/port_view.rs` during
//! step 3); the interface is fixed here.
//!
//! - A background scan (`core::ports::listening` over `Runner::pids`) every ~2 s, only
//!   while something runs or a port view is shown; results cached here, never computed
//!   during render (R1.3).
//! - Detected ports are remembered per command (`core::ports::PortMemory`, in the state
//!   folder); an empty scan never erases them.
//! - Before a start, a remembered or declared port that someone holds refuses the start
//!   with "port 5720 is held by …"; the toast offers "Stop holder": our own run stops at
//!   once; another process is stopped (SIGTERM) only after a confirmation that names it.
//! - `Services::ports` reads this cache (plus the demo's fake holders).

use crate::runner::RunKey;
use std::collections::BTreeMap;

/// What the last scan found, per run.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortScan {
    pub listening: BTreeMap<RunKey, Vec<u16>>,
}
