use crate::child::Signal;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// No scopes here: runs follow their process tree.
pub fn systemd_run() -> Option<&'static Path> {
    None
}

/// A scope's name, as a log written elsewhere may hold; it never holds anything here.
#[derive(Clone, Debug)]
pub struct Scope {
    pub unit: String,
}

impl Scope {
    pub fn named(unit: impl Into<String>) -> Scope {
        Scope { unit: unit.into() }
    }

    pub fn next(instance: &str) -> Scope {
        static N: AtomicU64 = AtomicU64::new(0);
        Scope::named(format!("devdeck-{instance}-{}", N.fetch_add(1, Ordering::Relaxed)))
    }

    pub fn command(&self, systemd_run: &Path, _unset: &[&str]) -> Command {
        Command::new(systemd_run)
    }

    pub fn pids(&self) -> Vec<u32> {
        Vec::new()
    }

    pub fn is_empty(&self) -> bool {
        true
    }

    pub fn signal(&self, _signal: Signal) {}

    pub fn kill(&self) {}
}
