//! Running a process (L1, U2, U3): signals to a process or its group, a session of its
//! own, a pseudo-terminal, and one wait for whichever comes first of the child's exit,
//! the terminal's output or room for input, and a wake-up from another thread.

use crate::{SysError, Support};
use std::os::fd::OwnedFd;
use std::process::{Child, Command};
use std::time::Duration;

pub(crate) mod unix;

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod imp;

/// Whether a terminal and its watch can be had here; a run is refused with the reason
/// before anything starts when they can't.
pub const SUPPORT: Support = imp::SUPPORT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Hup,
    Int,
    Term,
    Kill,
}

/// How a child ended: its code, or the signal that ended it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    Code(i32),
    Signal(i32),
}

/// Whether devdeck may signal `pid` or its group: never pid 0 or 1 (which would mean our
/// own group or every process), devdeck itself, nor devdeck's own group.
pub fn may_signal(pid: u32) -> bool {
    unix::may_signal(pid)
}

/// `signal` to `pid`; an error when it may not be signalled ([`may_signal`]) or the OS
/// refused.
pub fn signal(pid: u32, signal: Signal) -> std::io::Result<()> {
    unix::signal(pid, signal)
}

/// `signal` to the process group `pid` leads, when it may be signalled; errors ignored.
pub fn signal_group(pid: u32, signal: Signal) {
    unix::signal_group(pid, signal)
}

/// Whether the group `pid` leads still has a process (and may be signalled).
pub fn group_exists(pid: u32) -> bool {
    unix::group_exists(pid)
}

/// The command starts a session of its own: no terminal of devdeck's to take over, and
/// its group is its pid.
pub fn own_session(cmd: &mut Command) {
    unix::own_session(cmd)
}

/// The command starts a session of its own whose controlling terminal is its stdin (the
/// terminal's other side, given as its stdio).
pub fn terminal_session(cmd: &mut Command) {
    unix::terminal_session(cmd)
}

/// The side of a pseudo-terminal devdeck keeps.
pub struct Terminal(imp::Terminal);

impl Terminal {
    /// A new terminal of `rows` × `cols`, and its other side, for the process's stdio.
    pub fn open(rows: u16, cols: u16) -> Result<(Terminal, OwnedFd), SysError> {
        imp::Terminal::open(rows, cols).map(|(t, other)| (Terminal(t), other))
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        self.0.resize(rows, cols)
    }

    /// The process group in the terminal's foreground.
    pub fn foreground_group(&self) -> Option<u32> {
        self.0.foreground_group()
    }

    /// Reads and writes return at once (`WouldBlock`) from now on.
    pub fn set_nonblocking(&self) {
        self.0.set_nonblocking()
    }

    /// What the process wrote; 0 or an error once every process closed its side.
    pub fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }

    pub fn write(&self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.write(bytes)
    }
}

/// Watches a child (made right after it started, before anything could reap it).
pub struct Watch(imp::Watch);

/// Wakes a [`Watch`]'s wait from another thread. Never blocks; does nothing once the
/// watch is gone. Several wakes before the waiter drains them count as one.
pub struct Waker(imp::Waker);

/// What a wait found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ready {
    pub wake: bool,
    pub child: bool,
    /// Output to read, or the terminal hung up or failed (a read says which).
    pub readable: bool,
    /// Room for input.
    pub writable: bool,
}

impl Watch {
    pub fn new(child: &Child) -> Result<Watch, SysError> {
        imp::Watch::new(child).map(Watch)
    }

    pub fn waker(&self) -> Waker {
        Waker(self.0.waker())
    }

    /// Waits for a wake, for the child's end when `child`, for the terminal's output when
    /// one is given (and room for input when `out`), at most `timeout` (None: no limit).
    /// `Interrupted` means: wait again.
    pub fn wait(&self, terminal: Option<&Terminal>, out: bool, child: bool, timeout: Option<Duration>) -> std::io::Result<Ready> {
        self.0.wait(terminal.map(|t| &t.0), out, child, timeout)
    }

    /// Takes the wakes so far: the next wait waits for a new one.
    pub fn drain_wake(&self) {
        self.0.drain_wake()
    }

    /// The child's status, reaping it: waiting for its end when `wait`, else only once it
    /// ended. `Code(-1)` when it can't be known.
    pub fn reap(&self, wait: bool) -> ExitStatus {
        self.0.reap(wait)
    }
}

impl Waker {
    pub fn wake(&self) {
        self.0.wake()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devdeck_and_its_group_and_pids_0_and_1_are_never_signalled() {
        assert!(!may_signal(0));
        assert!(!may_signal(1));
        assert!(!may_signal(std::process::id()));
        assert!(!may_signal(rustix::process::getpgrp().as_raw_nonzero().get() as u32));
        let mut child = Command::new("sleep").arg("5").spawn().unwrap();
        assert!(may_signal(child.id()));
        signal(child.id(), Signal::Kill).unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn own_session_starts_a_session_and_a_group() {
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        own_session(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let pid = child.id();
        assert_eq!(crate::procs::get(pid).map(|p| p.session).unwrap_or(pid), pid);
        assert!(group_exists(pid));
        signal_group(pid, Signal::Kill);
        child.wait().unwrap();
        assert!(!group_exists(pid));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn says_why_nothing_runs_in_a_terminal() {
        assert!(matches!(Terminal::open(24, 80), Err(SysError::Unsupported(_))));
    }
}

/// Against real processes and terminals, on an OS with a backend (macOS from step 2c).
#[cfg(all(test, target_os = "linux"))]
mod conformance {
    use super::*;
    use std::process::Stdio;
    use std::time::Instant;

    /// `line` in a new terminal, with its watch.
    fn in_terminal(line: &str) -> (Terminal, Child, Watch) {
        let (term, other) = Terminal::open(24, 80).unwrap();
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", line]);
        let stdio = || Stdio::from(other.try_clone().unwrap());
        cmd.stdin(stdio()).stdout(stdio()).stderr(stdio());
        terminal_session(&mut cmd);
        let child = cmd.spawn().unwrap();
        drop(cmd);
        drop(other);
        let watch = Watch::new(&child).unwrap();
        term.set_nonblocking();
        (term, child, watch)
    }

    /// Output until `until` shows, and whether the child ended meanwhile; panics after 5 s.
    fn read_until(term: &Terminal, watch: &Watch, until: &str) -> String {
        let end = Instant::now() + Duration::from_secs(5);
        let mut out = String::new();
        let mut buf = [0u8; 4096];
        while !out.contains(until) {
            assert!(Instant::now() < end, "timed out; output so far: {out:?}");
            let ready = watch.wait(Some(term), false, false, Some(Duration::from_millis(100))).unwrap();
            if ready.readable
                && let Ok(n) = term.read(&mut buf)
            {
                out.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        }
        out
    }

    fn wait_child(watch: &Watch) {
        let ready = watch.wait(None, false, true, Some(Duration::from_secs(5))).unwrap();
        assert!(ready.child, "the child ends within 5 s");
    }

    #[test]
    fn a_child_in_a_terminal_reads_input_writes_output_and_ends_with_its_code() {
        let (term, child, watch) = in_terminal("stty size; read l; echo got:$l; exit 3");
        let out = read_until(&term, &watch, "24 80");
        assert!(out.contains("24 80"), "{out:?}");
        assert_eq!(term.foreground_group(), Some(child.id()), "the terminal is its controlling one");
        let mut written = 0;
        while written < 3 {
            written += term.write(&b"hi\n"[written..]).unwrap_or(0);
        }
        read_until(&term, &watch, "got:hi");
        wait_child(&watch);
        assert_eq!(watch.reap(false), ExitStatus::Code(3));
    }

    #[test]
    fn a_resize_reaches_the_child() {
        let (term, child, watch) = in_terminal("read l; stty size; read l");
        term.resize(30, 100);
        let _ = term.write(b"\n");
        read_until(&term, &watch, "30 100");
        signal_group(child.id(), Signal::Kill);
        wait_child(&watch);
    }

    #[test]
    #[expect(clippy::zombie_processes, reason = "reaped through its watch")]
    fn a_killed_child_reports_its_signal() {
        let child = Command::new("sleep").arg("5").spawn().unwrap();
        let watch = Watch::new(&child).unwrap();
        signal(child.id(), Signal::Kill).unwrap();
        wait_child(&watch);
        assert_eq!(watch.reap(true), ExitStatus::Signal(9));
    }

    #[test]
    fn wakes_count_as_one_until_drained() {
        let mut child = Command::new("sleep").arg("5").spawn().unwrap();
        let watch = Watch::new(&child).unwrap();
        let waker = watch.waker();
        for _ in 0..3 {
            waker.wake();
        }
        assert!(watch.wait(None, false, false, Some(Duration::ZERO)).unwrap().wake);
        watch.drain_wake();
        assert!(!watch.wait(None, false, false, Some(Duration::from_millis(10))).unwrap().wake, "drained");
        child.kill().unwrap();
        child.wait().unwrap();
        drop(watch);
        waker.wake();
    }

    #[test]
    #[expect(clippy::zombie_processes, reason = "reaped through its watch")]
    fn an_ended_child_is_not_watched_when_not_asked() {
        let child = Command::new("true").spawn().unwrap();
        let watch = Watch::new(&child).unwrap();
        wait_child(&watch);
        assert_eq!(watch.wait(None, false, false, Some(Duration::from_millis(10))).unwrap(), Ready::default());
        assert_eq!(watch.reap(false), ExitStatus::Code(0));
    }

    #[test]
    #[expect(clippy::zombie_processes, reason = "reaped through its watch")]
    fn a_terminal_nobody_holds_reads_as_closed() {
        let child = Command::new("true").stdout(Stdio::null()).spawn().unwrap();
        let watch = Watch::new(&child).unwrap();
        let (term, other) = Terminal::open(24, 80).unwrap();
        drop(other);
        term.set_nonblocking();
        let ready = watch.wait(Some(&term), false, false, Some(Duration::from_secs(1))).unwrap();
        assert!(ready.readable, "a hang-up");
        assert!(matches!(term.read(&mut [0u8; 16]), Ok(0) | Err(_)));
        watch.reap(true);
    }
}
