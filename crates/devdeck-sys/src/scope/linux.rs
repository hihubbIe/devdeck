//! systemd scopes: `systemd-run --user --scope`, found again under the user manager's
//! cgroup.

use crate::child::Signal;
use rustix::process::{Pid, PidfdFlags};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How long the check that scopes work may take (the first `systemd-run` takes ~40 ms).
const PROBE_WAIT: Duration = Duration::from_secs(2);
/// How deep under the user manager's cgroup a scope is looked for (`app.slice/x.scope`).
const DEPTH: usize = 4;

/// `systemd-run` when runs can get scopes here; checked once (start it early on a
/// thread: the first call waits for the check).
pub fn systemd_run() -> Option<&'static Path> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND.get_or_init(probe).as_deref()
}

/// cgroup v2 with `cgroup.kill`, `systemd-run` on the PATH, and a scope it really
/// starts where scopes are looked for ([`user_root`]).
fn probe() -> Option<PathBuf> {
    if !own_cgroup()?.join("cgroup.kill").exists() {
        return None;
    }
    let run = std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join("systemd-run")).find(|p| p.is_file())?;
    let unit = format!("devdeck-probe-{}", std::process::id());
    let mut child = Command::new(&run)
        .args(["--user", "--scope", "--quiet", "--collect", &format!("--unit={unit}"), "--", "cat", "/proc/self/cgroup"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let end = Instant::now() + PROBE_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut text = String::new();
                let _ = std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text);
                let path = text.lines().find_map(|l| l.strip_prefix("0::"))?;
                let found = Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/'));
                return (status.success() && found.starts_with(user_root())).then_some(run);
            }
            Ok(None) if Instant::now() < end => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// devdeck's own cgroup folder.
fn own_cgroup() -> Option<PathBuf> {
    let text = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = text.lines().find_map(|l| l.strip_prefix("0::"))?;
    Some(Path::new("/sys/fs/cgroup").join(path.trim_start_matches('/')))
}

/// Where the user manager keeps its units' cgroups.
fn user_root() -> PathBuf {
    let uid = rustix::process::getuid().as_raw();
    PathBuf::from(format!("/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service"))
}

/// One run's scope, by its unit name (`devdeck-<instance>-<n>`, without `.scope`).
#[derive(Clone, Debug)]
pub struct Scope {
    pub unit: String,
    /// Its cgroup folder once found (systemd creates it as the run starts).
    dir: OnceLock<PathBuf>,
}

impl Scope {
    pub fn named(unit: impl Into<String>) -> Scope {
        Scope { unit: unit.into(), dir: OnceLock::new() }
    }

    /// A new scope name for a run of the devdeck `instance`.
    pub fn next(instance: &str) -> Scope {
        static N: AtomicU64 = AtomicU64::new(0);
        Scope::named(format!("devdeck-{instance}-{}", N.fetch_add(1, Ordering::Relaxed)))
    }

    /// `systemd-run` starting the rest of the command (`/bin/sh -c …`, given after) in
    /// this scope: it keeps its pid, session and terminal. The program gets the
    /// environment given to `systemd-run` without what systemd adds (`INVOCATION_ID`) and
    /// without `unset` (what only `systemd-run` needed).
    pub fn command(&self, systemd_run: &Path, unset: &[&str]) -> Command {
        let mut cmd = Command::new(systemd_run);
        cmd.args(["--user", "--scope", "--quiet", "--collect", &format!("--unit={}", self.unit), "--", "/usr/bin/env", "-u", "INVOCATION_ID"]);
        for name in unset {
            cmd.args(["-u", name]);
        }
        cmd.arg("/bin/sh");
        cmd
    }

    /// Its cgroup folder, once systemd made it (gone again once empty).
    fn dir(&self) -> Option<&Path> {
        if let Some(d) = self.dir.get() {
            return Some(d);
        }
        let name = format!("{}.scope", self.unit);
        let found = find_dir(&user_root(), &name, DEPTH)?;
        Some(self.dir.get_or_init(|| found))
    }

    /// The processes in it now.
    pub fn pids(&self) -> Vec<u32> {
        let Some(dir) = self.dir() else { return Vec::new() };
        std::fs::read_to_string(dir.join("cgroup.procs")).unwrap_or_default().lines().filter_map(|l| l.trim().parse().ok()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.pids().is_empty()
    }

    /// `signal` to each process in it: through a pidfd, once the process is checked to be
    /// still in it (a pid read a moment ago may have been reused since).
    pub fn signal(&self, signal: Signal) {
        let suffix = format!("/{}.scope", self.unit);
        for pid in self.pids() {
            let Some(p) = Pid::from_raw(pid as i32) else { continue };
            let Ok(fd) = rustix::process::pidfd_open(p, PidfdFlags::empty()) else { continue };
            let inside = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).is_ok_and(|c| c.lines().any(|l| l.trim_end().ends_with(&suffix)));
            if inside {
                let _ = rustix::process::pidfd_send_signal(&fd, crate::child::unix::to_rustix(signal));
            }
        }
    }

    /// Kills everything in it at once, the ones forking meanwhile too (`cgroup.kill`).
    pub fn kill(&self) {
        if let Some(dir) = self.dir() {
            let _ = std::fs::write(dir.join("cgroup.kill"), "1");
        }
    }
}

/// The folder called `name` under `root`, `depth` levels down at most.
fn find_dir(root: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    let dirs: Vec<PathBuf> = entries.filter_map(Result::ok).filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
    if let Some(d) = dirs.iter().find(|d| d.file_name().is_some_and(|n| n == name)) {
        return Some(d.clone());
    }
    if depth == 0 {
        return None;
    }
    dirs.iter().find_map(|d| find_dir(d, name, depth - 1))
}
