//! Real processes for runs (R6.7, R8.2), in a pseudo-terminal from `alacritty_terminal`'s
//! `tty` (each in its own session and process group). Builder D of M2 step 2 implements
//! this file; the interface is fixed here.
//!
//! - Output is read on a thread and sent as bytes; the exit is sent once the output is
//!   drained (with a timeout: a grandchild may keep the terminal open), with the code or
//!   the signal number.
//! - Stop: ^C (SIGINT) to the process group, then SIGTERM, then SIGKILL, each after its
//!   [`StopKind`]'s wait; descendants that left the group (found through `procfs`) get
//!   the same. Dropping a [`Process`] kills it at once.
//! - Every started group is recorded in the state folder ([`GroupLog`]) until it exits,
//!   so a devdeck that was killed can offer to kill what it left behind at next start.
//! - [`kill_all`] is safe to call from a signal-handling thread (devdeck receiving
//!   SIGTERM, SIGHUP or SIGINT).

use crate::apps::Command;
use crate::runs::Exit;
use crate::terminal::TermSize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long each stop signal gets before the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopKind {
    /// Dev servers, builds: ^C, then 3 s, SIGTERM, then 2 s, SIGKILL.
    Default,
    /// docker compose stops its containers on ^C (10 s by default each): ^C, then 15 s,
    /// SIGTERM, then 5 s, SIGKILL.
    Compose,
}

impl StopKind {
    /// docker compose commands get [`StopKind::Compose`].
    pub fn of(cmd: &Command) -> StopKind {
        if cmd.source() == "compose" { StopKind::Compose } else { StopKind::Default }
    }

    /// The whole stop sequence's length: quitting waits this long before killing.
    pub fn total(self) -> Duration {
        let (a, b) = self.waits();
        a + b
    }

    pub fn waits(self) -> (Duration, Duration) {
        match self {
            StopKind::Default => (Duration::from_secs(3), Duration::from_secs(2)),
            StopKind::Compose => (Duration::from_secs(15), Duration::from_secs(5)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spawn {
    /// Run with `sh -c`.
    pub line: String,
    pub dir: PathBuf,
    /// The whole environment (the toolbox's plus the run's own), `TERM` and
    /// `COLORTERM` included.
    pub vars: Vec<(String, String)>,
    pub size: TermSize,
    pub stop: StopKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProcessEvent {
    Output(Vec<u8>),
    /// The last event.
    Exited(Exit),
}

/// A running process. Dropping it kills its whole tree.
pub struct Process {
    _private: (),
}

impl Process {
    /// Starts `spawn`; its output and exit arrive on the receiver.
    pub fn spawn(spawn: Spawn, log: Option<GroupLog>) -> Result<(Process, async_channel::Receiver<ProcessEvent>), String> {
        let _ = (spawn, log);
        todo!("M2 step 2, builder D")
    }

    /// The process id, which is also its process group id.
    pub fn pid(&self) -> u32 {
        todo!("M2 step 2, builder D")
    }

    pub fn write(&self, bytes: &[u8]) {
        let _ = bytes;
        todo!("M2 step 2, builder D")
    }

    pub fn resize(&self, size: TermSize) {
        let _ = size;
        todo!("M2 step 2, builder D")
    }

    /// Starts the stop sequence (returns at once; the exit arrives as an event).
    pub fn stop(&self) {
        todo!("M2 step 2, builder D")
    }
}

/// Process groups devdeck started and that may still run, in a file of the state folder.
#[derive(Clone, Debug)]
pub struct GroupLog {
    pub path: PathBuf,
}

impl GroupLog {
    pub fn new(state_dir: &Path) -> GroupLog {
        GroupLog { path: state_dir.join("processes") }
    }

    /// Groups recorded by an earlier devdeck that still run (same pid and start time).
    pub fn leftovers(&self) -> Vec<u32> {
        todo!("M2 step 2, builder D")
    }
}

/// Kills these process groups and their escaped descendants (SIGKILL), now.
pub fn kill_all(pids: &[u32]) {
    let _ = pids;
    todo!("M2 step 2, builder D")
}
