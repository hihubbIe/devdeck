//! A run's processes kept together by the kernel (R8.2): each run starts in a systemd
//! scope of its own (`systemd-run --user --scope`), a cgroup its descendants stay in
//! whatever they do (`setsid`, a daemon's double fork, a cleared environment). A stop
//! signals what's in it and empties it (`cgroup.kill`); the scopes a devdeck left are
//! found again by name. Only where a systemd user manager and cgroup v2 with
//! `cgroup.kill` (Linux 5.14) are there ([`systemd_run`]); elsewhere a run follows its
//! process tree, as before.


#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod imp;

pub use imp::{Scope, systemd_run};

/// What `systemd-run` needs from devdeck's environment to reach the user manager: lent
/// to it when a run's own environment lacks them, and unset again for the run.
pub const BORROWED: &[&str] = &["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"];

/// Against a real systemd user manager; skipped where there is none.
#[cfg(test)]
mod conformance {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_until(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_process_in_a_scope_is_found_signalled_and_killed_with_it() {
        let Some(run) = systemd_run() else {
            eprintln!("no systemd user manager here: scope test skipped");
            return;
        };
        let scope = Scope::next(&format!("test{}", std::process::id()));
        let mut cmd = scope.command(run, &[]);
        cmd.args(["-c", "exec sleep 300"]);
        let mut child = cmd.spawn().expect("systemd-run starts");
        let pid = child.id();
        wait_until("the scope to hold the process", || scope.pids().contains(&pid));
        assert!(Scope::named(scope.unit.clone()).pids().contains(&pid), "found again by name");
        scope.signal(crate::child::Signal::Term);
        let status = child.wait().unwrap();
        assert!(!status.success(), "ended by the signal");
        wait_until("the scope to empty", || scope.is_empty());
    }

    #[test]
    fn killing_a_scope_ends_everything_in_it() {
        let Some(run) = systemd_run() else { return };
        let scope = Scope::next(&format!("test{}", std::process::id()));
        let mut cmd = scope.command(run, &[]);
        cmd.args(["-c", "sleep 300 & exec sleep 300"]);
        let mut child = cmd.spawn().expect("systemd-run starts");
        wait_until("both processes in the scope", || scope.pids().len() >= 2);
        scope.kill();
        let _ = child.wait();
        wait_until("the scope to empty", || scope.is_empty());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn scopes_are_never_there() {
        assert!(systemd_run().is_none());
        assert!(Scope::named("x").is_empty());
    }
}
