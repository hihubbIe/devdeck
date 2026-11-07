//! Listening ports (R6.9): which ports a run's processes listen on, who holds a port
//! before a start, and the ports each command used last time. Builder E of M2 step 2
//! implements this file; the interface is fixed here.
//!
//! Sockets come from `/proc/net/tcp` and `tcp6` (state LISTEN; IPv4-mapped IPv6
//! included; `0.0.0.0` and `127.0.0.1` on the same port conflict). They are matched to
//! processes through `/proc/<pid>/fd` links, for the given pids only (a whole-system
//! scan is thousands of reads). A holder we can't read (another user, docker-proxy as
//! root) is named as such.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Listening {
    pub port: u16,
    pub pid: u32,
}

/// Who holds a port.
#[derive(Clone, Debug, PartialEq)]
pub enum Holder {
    /// A process we can see: its pid and command line.
    Process { pid: u32, command: String },
    /// Listening, but the process isn't readable (another user).
    Unknown,
}

/// The ports `pids` listen on.
pub fn listening(pids: &[u32]) -> Vec<Listening> {
    let _ = pids;
    todo!("M2 step 2, builder E")
}

/// Who listens on `port` now, if anyone.
pub fn holder(port: u16) -> Option<Holder> {
    let _ = port;
    todo!("M2 step 2, builder E")
}

/// Ports each command listened on last time, in the state folder: a conflict can be
/// reported before a start even when no config names the port. Nested by workspace id,
/// app id, command id (app ids hold `/`, so no joined key).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortMemory {
    pub path: PathBuf,
    pub ports: BTreeMap<String, BTreeMap<String, BTreeMap<String, Vec<u16>>>>,
}

impl PortMemory {
    pub fn load(state_dir: &Path) -> PortMemory {
        let _ = state_dir;
        todo!("M2 step 2, builder E")
    }

    pub fn get(&self, workspace: &str, app: &str, cmd: &str) -> &[u16] {
        let _ = (workspace, app, cmd);
        todo!("M2 step 2, builder E")
    }

    /// Saves at once when they changed.
    pub fn remember(&mut self, workspace: &str, app: &str, cmd: &str, ports: &[u16]) -> Result<(), String> {
        let _ = (workspace, app, cmd, ports);
        todo!("M2 step 2, builder E")
    }
}
