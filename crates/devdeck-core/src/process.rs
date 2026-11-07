//! Real processes for runs (R6.7, R8.2), each in a pseudo-terminal, in its own session and
//! process group (the pid is the group id). Builder D of M2 step 2 implements this file;
//! the interface is fixed here.
//!
//! - Output is read on a thread and sent as bytes; the exit is sent once the output is
//!   drained (with a timeout: a grandchild may keep the terminal open), with the code or
//!   the signal number.
//! - Stop: ^C (SIGINT) to the process group, then SIGTERM, then SIGKILL, each after its
//!   [`StopKind`]'s wait; descendants that left the group (found through `procfs`) get
//!   SIGTERM and SIGKILL too (a terminal's ^C never reaches them either). It ends once
//!   nothing is left. Dropping a [`Process`] kills it at once.
//! - Every started group is recorded in the state folder ([`GroupLog`]) until it exits,
//!   so a devdeck that was killed can offer to kill what it left behind at next start.
//! - [`kill_all`] is safe to call from a signal-handling thread (devdeck receiving
//!   SIGTERM, SIGHUP or SIGINT).
//!
//! The terminal is opened here rather than with `alacritty_terminal`'s `tty`, which does
//! the same (`openpty`, `setsid`, `TIOCSCTTY`) but always passes devdeck's own environment
//! on, ignores a missing folder, and blocks in its `Drop` until the child exits. One
//! thread per run polls the terminal, the child's pidfd and a wake-up for input.

use crate::apps::Command;
use crate::procfs;
use crate::runs::Exit;
use crate::terminal::TermSize;
use rustix::event::{EventfdFlags, PollFd, PollFlags, Timespec};
use rustix::fd::{AsFd, OwnedFd};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, WaitIdStatus};
use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the exit waits for the rest of the output once the process ended: a
/// grandchild may keep the terminal open.
const DRAIN: Duration = Duration::from_millis(1500);
/// Output chunks waiting for the app at most. When the app falls behind, reading pauses
/// and the process blocks on its output, as in any terminal.
const QUEUE: usize = 64;
/// How often a stop checks whether everything ended.
const TICK: Duration = Duration::from_millis(20);

/// How long each stop signal gets before the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopKind {
    /// Dev servers, builds: ^C, then 3 s, SIGTERM, then 2 s, SIGKILL.
    Default,
    /// docker compose stops its containers on ^C (10 s by default each): ^C, then 15 s,
    /// SIGTERM, then 5 s, SIGKILL.
    Compose,
    /// Tests: 300 ms each.
    #[cfg(test)]
    Quick,
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
            #[cfg(test)]
            StopKind::Quick => (Duration::from_millis(300), Duration::from_millis(300)),
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
    tree: Arc<Tree>,
    stop: StopKind,
    master: Arc<OwnedFd>,
    input: Arc<Input>,
}

/// Bytes for the process, handed to its thread so a process that doesn't read never
/// blocks the caller.
struct Input {
    bytes: Mutex<Vec<u8>>,
    /// An eventfd that wakes the thread.
    wake: OwnedFd,
}

impl Process {
    /// Starts `spawn`; its output and exit arrive on the receiver.
    pub fn spawn(spawn: Spawn, log: Option<GroupLog>) -> Result<(Process, async_channel::Receiver<ProcessEvent>), String> {
        if !spawn.dir.is_dir() {
            return Err(format!("{}: no such folder", spawn.dir.display()));
        }
        let (master, slave) = open_terminal(spawn.size).map_err(|e| format!("no pseudo-terminal: {e}"))?;
        let wake = rustix::event::eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)
            .map_err(|e| format!("no eventfd: {e}"))?;
        let stdio = || slave.try_clone().map(std::process::Stdio::from).map_err(|e| e.to_string());
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.arg("-c").arg(&spawn.line).current_dir(&spawn.dir).env_clear().envs(spawn.vars.iter().map(|(k, v)| (k, v)));
        cmd.stdin(stdio()?).stdout(stdio()?).stderr(stdio()?);
        // SAFETY: between fork and exec, only system calls (no allocation, no locks). The
        // terminal is already on 0, 1 and 2: the new session takes it as its own.
        unsafe {
            cmd.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())?;
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| format!("couldn't start sh in {}: {e}", spawn.dir.display()))?;
        // Our copies of the terminal's other end close, so the output ends when the
        // process tree closes its own.
        drop(cmd);
        drop(slave);
        let pid = child.id();
        let start = stat(pid).map_or(0, |s| s.start);
        let tree = Arc::new(Tree { pid, start, stopping: AtomicBool::new(false), seen: Mutex::new(BTreeMap::new()) });
        // std's `Child` neither waits nor kills when dropped: the thread reaps it.
        let pidfd = match rustix::process::pidfd_open(Pid::from_child(&child), rustix::process::PidfdFlags::empty()) {
            Ok(fd) => fd,
            Err(e) => {
                tree.signal_all(Signal::KILL);
                let mut child = child;
                let _ = child.wait();
                return Err(format!("no pidfd: {e}"));
            }
        };
        let master = Arc::new(master);
        let input = Arc::new(Input { bytes: Mutex::new(Vec::new()), wake });
        let (tx, rx) = async_channel::bounded(QUEUE);
        let pump = Pump { pid, start, master: master.clone(), pidfd, input: input.clone(), tx, log };
        let thread = std::thread::Builder::new().name(format!("run {pid}")).spawn(move || pump.run());
        let process = Process { tree, stop: spawn.stop, master, input };
        if let Err(e) = thread {
            return Err(format!("no thread: {e}")); // dropping `process` kills it
        }
        Ok((process, rx))
    }

    /// The process id, which is also its process group id.
    pub fn pid(&self) -> u32 {
        self.tree.pid
    }

    pub fn write(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.input.bytes.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(bytes);
        let _ = rustix::io::write(&self.input.wake, &1u64.to_ne_bytes());
    }

    pub fn resize(&self, size: TermSize) {
        let _ = rustix::termios::tcsetwinsize(&*self.master, winsize(size));
    }

    /// Starts the stop sequence (returns at once; the exit arrives as an event).
    pub fn stop(&self) {
        if self.tree.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        let tree = self.tree.clone();
        let waits = self.stop.waits();
        let name = format!("stop {}", tree.pid);
        if std::thread::Builder::new().name(name).spawn(move || tree.stop(waits)).is_err() {
            self.tree.collect();
            self.tree.signal_all(Signal::KILL);
        }
    }
}

impl Drop for Process {
    /// Kills everything on a thread: finding the processes that left the group reads all
    /// of `/proc` (not for the caller, the UI), and must come before the kill, which
    /// would orphan them out of reach.
    fn drop(&mut self) {
        let tree = self.tree.clone();
        let name = format!("drop {}", tree.pid);
        let finish = move || {
            tree.collect();
            tree.signal_all(Signal::KILL);
        };
        if std::thread::Builder::new().name(name).spawn(finish.clone()).is_err() {
            finish();
        }
    }
}

/// A new terminal of `size`: the side devdeck keeps and the side the process gets.
fn open_terminal(size: TermSize) -> rustix::io::Result<(OwnedFd, OwnedFd)> {
    use rustix::pty::OpenptFlags;
    let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)?;
    rustix::pty::grantpt(&master)?;
    rustix::pty::unlockpt(&master)?;
    let name = rustix::pty::ptsname(&master, Vec::new())?;
    use rustix::fs::{Mode, OFlags};
    let slave = rustix::fs::open(name.as_c_str(), OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC, Mode::empty())?;
    rustix::termios::tcsetwinsize(&master, winsize(size))?;
    if let Ok(mut t) = rustix::termios::tcgetattr(&master) {
        t.input_modes.insert(rustix::termios::InputModes::IUTF8);
        let _ = rustix::termios::tcsetattr(&master, rustix::termios::OptionalActions::Now, &t);
    }
    Ok((master, slave))
}

fn winsize(size: TermSize) -> rustix::termios::Winsize {
    rustix::termios::Winsize { ws_row: size.rows, ws_col: size.cols, ws_xpixel: 0, ws_ypixel: 0 }
}

/// A run's thread: output out, input in, the exit once the output is drained.
struct Pump {
    pid: u32,
    start: u64,
    master: Arc<OwnedFd>,
    pidfd: OwnedFd,
    input: Arc<Input>,
    tx: async_channel::Sender<ProcessEvent>,
    log: Option<GroupLog>,
}

impl Pump {
    fn run(self) {
        // Here rather than in `spawn`: the file is synced to disk, which can take a while.
        if let Some(log) = &self.log {
            log.add(self.pid, self.start);
        }
        let _ = rustix::io::ioctl_fionbio(&*self.master, true);
        let mut buf = vec![0u8; 64 * 1024];
        let mut pending: Vec<u8> = Vec::new();
        // Some process still has the terminal open.
        let mut open = true;
        let mut exit: Option<(Exit, Instant)> = None;
        loop {
            if let Some((code, at)) = &exit
                && (!open || at.elapsed() >= DRAIN)
            {
                // Fails only when nobody listens any more.
                let _ = self.tx.send_blocking(ProcessEvent::Exited(code.clone()));
                return;
            }
            let (child, term, wake) = {
                let mut fds = vec![PollFd::new(&self.input.wake, PollFlags::IN)];
                if exit.is_none() {
                    fds.push(PollFd::new(&self.pidfd, PollFlags::IN));
                }
                if open {
                    let out = if pending.is_empty() { PollFlags::empty() } else { PollFlags::OUT };
                    fds.push(PollFd::new(&*self.master, PollFlags::IN | out));
                }
                let timeout = exit.as_ref().map(|(_, at)| timespec(DRAIN.saturating_sub(at.elapsed())));
                match rustix::event::poll(&mut fds, timeout.as_ref()) {
                    Ok(_) => {}
                    Err(Errno::INTR) => continue,
                    Err(_) => {
                        // Can't watch any more: report the exit when it comes.
                        let code = exit.map(|(c, _)| c).unwrap_or_else(|| self.reap(WaitIdOptions::EXITED));
                        let _ = self.tx.send_blocking(ProcessEvent::Exited(code));
                        return;
                    }
                }
                let wake = fds[0].revents();
                let child = if exit.is_none() { fds[1].revents() } else { PollFlags::empty() };
                let term = if open { fds[fds.len() - 1].revents() } else { PollFlags::empty() };
                (child, term, wake)
            };
            if !wake.is_empty() {
                let _ = rustix::io::read(&self.input.wake, &mut [0u8; 8]);
                pending.append(&mut self.input.bytes.lock().unwrap_or_else(|e| e.into_inner()));
            }
            if term.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR) {
                match rustix::io::read(&*self.master, &mut buf[..]) {
                    Ok(0) => open = false,
                    // An error means nobody listens any more: keep reading so the process
                    // never blocks on its output.
                    Ok(n) => drop(self.tx.send_blocking(ProcessEvent::Output(buf[..n].to_vec()))),
                    Err(Errno::AGAIN | Errno::INTR) => {}
                    // EIO: every process closed the terminal.
                    Err(_) => open = false,
                }
            }
            if !open {
                pending.clear();
            } else if term.contains(PollFlags::OUT) {
                match rustix::io::write(&*self.master, &pending) {
                    Ok(n) => drop(pending.drain(..n)),
                    Err(Errno::AGAIN | Errno::INTR) => {}
                    Err(_) => pending.clear(),
                }
            }
            if !child.is_empty() {
                let code = self.reap(WaitIdOptions::EXITED | WaitIdOptions::NOHANG);
                exit = Some((code, Instant::now()));
                if let Some(log) = &self.log {
                    log.remove(self.pid);
                }
            }
        }
    }

    /// The child's exit status, reaping it. `-1` when it can't be known.
    fn reap(&self, options: WaitIdOptions) -> Exit {
        match rustix::process::waitid(WaitId::PidFd(self.pidfd.as_fd()), options) {
            Ok(Some(status)) => exit_of(&status),
            _ => Exit::Code(-1),
        }
    }
}

fn exit_of(status: &WaitIdStatus) -> Exit {
    match status.terminating_signal() {
        Some(signal) => Exit::Signal(signal),
        None => Exit::Code(status.exit_status().unwrap_or(-1)),
    }
}

fn timespec(d: Duration) -> Timespec {
    Timespec { tv_sec: d.as_secs() as _, tv_nsec: d.subsec_nanos() as _ }
}

/// A run's processes: its group, and every descendant seen so far, which may have left
/// the group or lost their parent since.
struct Tree {
    pid: u32,
    /// The leader's start time: a pid reused after it ended isn't ours.
    start: u64,
    stopping: AtomicBool,
    /// pid → start time.
    seen: Mutex<BTreeMap<u32, u64>>,
}

impl Tree {
    /// ^C, then SIGTERM, then SIGKILL, each after its wait; ends once nothing is left.
    fn stop(&self, waits: (Duration, Duration)) {
        self.collect();
        self.signal_group(Signal::INT);
        if self.gone_within(waits.0) {
            return;
        }
        self.collect();
        self.signal_all(Signal::TERM);
        if self.gone_within(waits.1) {
            return;
        }
        self.collect();
        self.signal_all(Signal::KILL);
    }

    /// Adds the live processes below the leader and below those seen before (an orphan's
    /// children are still below it), and forgets those that ended.
    fn collect(&self) {
        let pairs = procfs::parents();
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|pid, start| alive(*pid, *start));
        let mut roots: Vec<u32> = seen.keys().copied().collect();
        if alive(self.pid, self.start) {
            roots.push(self.pid);
        }
        for root in roots {
            for pid in procfs::tree(root, &pairs) {
                if pid != self.pid && !seen.contains_key(&pid)
                    && let Some(s) = stat(pid)
                {
                    seen.insert(pid, s.start);
                }
            }
        }
    }

    /// The group with our leader's id is still ours. The kernel doesn't reuse an id while
    /// a group of that id has members, so it can only belong to someone else once a new
    /// process holds the leader's pid: one with another start time.
    fn group_is_ours(&self) -> bool {
        stat(self.pid).is_none_or(|s| s.start == self.start)
    }

    fn signal_group(&self, signal: Signal) {
        if self.group_is_ours() {
            signal_group(self.pid, signal);
        }
    }

    fn signal_all(&self, signal: Signal) {
        self.signal_group(signal);
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        for (pid, start) in seen.iter() {
            if alive(*pid, *start) {
                signal_one(*pid, signal);
            }
        }
    }

    fn gone(&self) -> bool {
        let group = self.group_is_ours() && Pid::from_raw(self.pid as i32).is_some_and(|p| rustix::process::test_kill_process_group(p).is_ok());
        !group && self.seen.lock().unwrap_or_else(|e| e.into_inner()).iter().all(|(pid, start)| !alive(*pid, *start))
    }

    fn gone_within(&self, wait: Duration) -> bool {
        let end = Instant::now() + wait;
        loop {
            if self.gone() {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(TICK);
        }
    }
}

/// Never pid 0 or 1 (which would mean our own group or every process) nor devdeck itself.
fn ours(pid: u32) -> Option<Pid> {
    let p = Pid::from_raw(i32::try_from(pid).ok()?)?;
    let me = rustix::process::getpid();
    (pid > 1 && p != me && p != rustix::process::getpgrp()).then_some(p)
}

fn signal_group(pid: u32, signal: Signal) {
    if let Some(p) = ours(pid) {
        let _ = rustix::process::kill_process_group(p, signal);
    }
}

fn signal_one(pid: u32, signal: Signal) {
    if let Some(p) = ours(pid) {
        let _ = rustix::process::kill_process(p, signal);
    }
}

struct Stat {
    state: char,
    start: u64,
}

/// The state and start time (in clock ticks since boot) of `pid`.
fn stat(pid: u32) -> Option<Stat> {
    parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// Fields are counted after the command name's last `)` (see `procfs::parse_ppid`):
/// state is field 3, start time field 22.
fn parse_stat(text: &str) -> Option<Stat> {
    let mut fields = text.rsplit_once(')')?.1.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let start = fields.nth(18)?.parse().ok()?;
    Some(Stat { state, start })
}

/// `pid` runs and is the process that started at `start` (zombies have ended).
fn alive(pid: u32, start: u64) -> bool {
    stat(pid).is_some_and(|s| s.start == start && !matches!(s.state, 'Z' | 'X'))
}

/// Process groups devdeck started and that may still run, in a file of the state folder.
#[derive(Clone, Debug)]
pub struct GroupLog {
    pub path: PathBuf,
}

/// Several runs start and end at once: the file is read and rewritten under this lock.
static LOG_LOCK: Mutex<()> = Mutex::new(());

impl GroupLog {
    pub fn new(state_dir: &Path) -> GroupLog {
        GroupLog { path: state_dir.join("processes") }
    }

    /// Groups recorded by an earlier devdeck that still run (same pid and start time).
    /// Our own children aren't leftovers.
    pub fn leftovers(&self) -> Vec<u32> {
        let me = std::process::id();
        self.read().into_iter().filter(|(pid, start)| alive(*pid, *start) && procfs::parent_of(*pid) != Some(me)).map(|(pid, _)| pid).collect()
    }

    /// Every recorded group still running, ours included: what to kill when devdeck
    /// itself is told to quit (a signal-handling thread can't reach the runner).
    pub fn live(&self) -> Vec<u32> {
        self.read().into_iter().filter(|(pid, start)| alive(*pid, *start)).map(|(pid, _)| pid).collect()
    }

    /// `pid start` lines; anything else is skipped.
    fn read(&self) -> Vec<(u32, u64)> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        text.lines()
            .filter_map(|l| {
                let (pid, start) = l.split_once(' ')?;
                Some((pid.parse().ok()?, start.parse().ok()?))
            })
            .collect()
    }

    /// Rewrites the file with `change` applied, dropping entries that ended. Best effort:
    /// a run starts even when its group can't be recorded.
    fn update(&self, change: impl FnOnce(&mut Vec<(u32, u64)>)) {
        let _lock = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.read();
        entries.retain(|(pid, start)| alive(*pid, *start));
        change(&mut entries);
        let text: String = entries.iter().map(|(pid, start)| format!("{pid} {start}\n")).collect();
        let _ = crate::state::write_atomic(&self.path, &text);
    }

    fn add(&self, pid: u32, start: u64) {
        self.update(|e| e.push((pid, start)));
    }

    fn remove(&self, pid: u32) {
        self.update(|e| e.retain(|(p, _)| *p != pid));
    }
}

/// Kills these process groups and their escaped descendants (SIGKILL), now. Reads `/proc`
/// and allocates: call it from a thread (signal-hook's iterator), not a signal handler.
pub fn kill_all(pids: &[u32]) {
    // Descendants first: once a leader is killed, its children move to another parent.
    let pairs = procfs::parents();
    let all: BTreeSet<u32> = pids.iter().filter(|p| ours(**p).is_some()).flat_map(|p| procfs::tree(*p, &pairs)).collect();
    for pid in pids {
        signal_group(*pid, Signal::KILL);
    }
    for pid in all {
        signal_one(pid, Signal::KILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_channel::Receiver;

    const SIZE: TermSize = TermSize { cols: 80, rows: 24 };

    fn spawn_with(line: &str, vars: Vec<(String, String)>) -> (Process, Receiver<ProcessEvent>) {
        let spawn = Spawn { line: line.into(), dir: std::env::temp_dir(), vars, size: SIZE, stop: StopKind::Quick };
        Process::spawn(spawn, None).expect("spawns")
    }

    /// With the test's PATH only (for `sleep`, `setsid`, `stty`).
    fn spawn(line: &str) -> (Process, Receiver<ProcessEvent>) {
        spawn_with(line, vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())])
    }

    /// Output until `until` shows up, or the exit (`None` while still running). Panics
    /// after 5 s.
    fn read(rx: &Receiver<ProcessEvent>, out: &mut String, until: Option<&str>) -> Option<Exit> {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            if until.is_some_and(|u| out.contains(u)) {
                return None;
            }
            match rx.try_recv() {
                Ok(ProcessEvent::Output(b)) => out.push_str(&String::from_utf8_lossy(&b)),
                Ok(ProcessEvent::Exited(e)) => return Some(e),
                Err(_) => {
                    assert!(Instant::now() < end, "timed out; output so far: {out:?}");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    fn run_to_end(line: &str) -> (String, Exit) {
        let (_p, rx) = spawn(line);
        let mut out = String::new();
        let exit = read(&rx, &mut out, None).expect("exits");
        (out, exit)
    }

    /// The pid printed after `marker ` in `out`.
    fn pid_after(out: &str, marker: &str) -> u32 {
        let rest = &out[out.find(marker).expect("marker") + marker.len()..];
        rest.split_whitespace().next().and_then(|n| n.parse().ok()).expect("pid")
    }

    fn session_of(pid: u32) -> Option<u32> {
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        text.rsplit_once(')')?.1.split_whitespace().nth(3)?.parse().ok()
    }

    fn wait_until(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(3);
        while !f() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Kills processes the test started itself, even when it fails.
    struct Reap(Vec<(u32, u64)>);

    impl Drop for Reap {
        fn drop(&mut self) {
            for (pid, start) in &self.0 {
                if alive(*pid, *start) {
                    signal_one(*pid, Signal::KILL);
                }
            }
        }
    }

    #[test]
    fn output_arrives_then_the_exit_code() {
        let (out, exit) = run_to_end("printf 'hello\\n'; exit 3");
        assert!(out.contains("hello"), "{out:?}");
        assert_eq!(exit, Exit::Code(3));
    }

    #[test]
    fn a_process_killed_by_a_signal_reports_its_number() {
        assert_eq!(run_to_end("kill -KILL $$").1, Exit::Signal(9));
    }

    #[test]
    fn a_missing_folder_is_a_spawn_failure() {
        let dir = std::env::temp_dir().join("devdeck-no-such-folder");
        let spawn = Spawn { line: "true".into(), dir: dir.clone(), vars: vec![], size: SIZE, stop: StopKind::Quick };
        let err = Process::spawn(spawn, None).err().expect("fails");
        assert!(err.contains(&dir.display().to_string()), "{err}");
    }

    #[test]
    fn the_environment_is_exactly_the_one_given() {
        // A variable of this test process that the shell doesn't set itself.
        let shell_own = ["PATH", "PWD", "OLDPWD", "IFS", "PS1", "PS2", "PS4", "OPTIND", "PPID", "SHLVL", "_"];
        let (name, _) = std::env::vars()
            .find(|(k, _)| !shell_own.contains(&k.as_str()) && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .expect("the test has an environment");
        let (_p, rx) = spawn_with(&format!("echo \"[$ONLY][${{{name}-unset}}]\""), vec![("ONLY".into(), "yes".into())]);
        let mut out = String::new();
        assert_eq!(read(&rx, &mut out, None), Some(Exit::Code(0)));
        assert!(out.contains("[yes][unset]"), "{name} leaked: {out:?}");
    }

    #[test]
    fn input_and_size_reach_the_process() {
        let (p, rx) = spawn("echo ready; read x; echo \"got $x\"; stty size");
        let mut out = String::new();
        read(&rx, &mut out, Some("ready"));
        p.resize(TermSize { cols: 100, rows: 40 });
        p.write(b"abc\n");
        assert_eq!(read(&rx, &mut out, None), Some(Exit::Code(0)));
        assert!(out.contains("got abc") && out.contains("40 100"), "{out:?}");
    }

    #[test]
    fn stop_ends_early_when_the_process_exits_on_ctrl_c() {
        let (p, rx) = spawn("echo ready; sleep 30");
        let mut out = String::new();
        read(&rx, &mut out, Some("ready"));
        p.stop();
        // Ended by the ^C itself, not by the SIGTERM or SIGKILL that would follow.
        assert_eq!(read(&rx, &mut out, None), Some(Exit::Signal(2)));
    }

    #[test]
    fn stop_ends_in_sigkill_when_signals_are_ignored() {
        let (p, rx) = spawn("trap '' INT TERM; echo ready; sleep 30");
        let mut out = String::new();
        read(&rx, &mut out, Some("ready"));
        let at = Instant::now();
        p.stop();
        assert_eq!(read(&rx, &mut out, None), Some(Exit::Signal(9)));
        assert!(at.elapsed() >= StopKind::Quick.total(), "{:?}", at.elapsed());
    }

    /// A child in its own session, as the run prints it: `(process, its pid, its start)`.
    fn with_escaped_child() -> (Process, Receiver<ProcessEvent>, u32, u64) {
        let (p, rx) = spawn("setsid sleep 30 & echo \"child $!\"; wait");
        let mut out = String::new();
        read(&rx, &mut out, Some("child "));
        read(&rx, &mut out, Some("\n"));
        let child = pid_after(&out, "child ");
        let start = stat(child).expect("child runs").start;
        wait_until("setsid", || session_of(child) == Some(child));
        (p, rx, child, start)
    }

    #[test]
    fn stop_kills_a_child_that_left_the_group() {
        let (p, rx, child, start) = with_escaped_child();
        let _reap = Reap(vec![(child, start)]);
        p.stop();
        read(&rx, &mut String::new(), None).expect("exits");
        wait_until("the escaped child to end", || !alive(child, start));
    }

    #[test]
    fn drop_kills_a_child_that_left_the_group() {
        let (p, _rx, child, start) = with_escaped_child();
        let _reap = Reap(vec![(child, start)]);
        drop(p);
        wait_until("the escaped child to end", || !alive(child, start));
    }

    #[test]
    fn the_exit_comes_after_a_timeout_when_a_grandchild_keeps_the_terminal() {
        // It ignores the SIGHUP the terminal sends when the session's leader exits.
        let (p, rx) = spawn("(trap '' HUP; sleep 30 & echo \"child $!\")");
        let mut out = String::new();
        read(&rx, &mut out, Some("child "));
        read(&rx, &mut out, Some("\n"));
        let child = pid_after(&out, "child ");
        let start = stat(child).expect("child runs").start;
        let _reap = Reap(vec![(child, start)]);
        let at = Instant::now();
        assert_eq!(read(&rx, &mut out, None), Some(Exit::Code(0)));
        assert!(at.elapsed() >= DRAIN - Duration::from_millis(200), "{:?}", at.elapsed());
        // Still in the group after its leader ended: dropping kills it.
        drop(p);
        wait_until("the grandchild to end", || !alive(child, start));
    }

    #[test]
    fn stat_fields_survive_odd_command_names() {
        let line = "42 (a) b) S 1 42 42 0 -1 4194560 0 0 0 0 0 0 0 0 20 0 1 0 98765 0 0";
        let s = parse_stat(line).expect("parses");
        assert_eq!((s.state, s.start), ('S', 98765));
        assert!(parse_stat("garbage").is_none());
    }

    #[test]
    fn the_group_log_keeps_runs_until_they_exit_and_finds_leftovers() {
        let dir = std::env::temp_dir().join(format!("devdeck-process-test-{}", std::process::id()));
        let log = GroupLog::new(&dir);

        // A run is recorded while it runs, isn't a leftover of ours, and is removed at exit.
        let spawn = Spawn { line: "echo ready; read x".into(), dir: dir.parent().unwrap().into(), vars: vec![], size: SIZE, stop: StopKind::Quick };
        let (p, rx) = Process::spawn(spawn, Some(log.clone())).expect("spawns");
        read(&rx, &mut String::new(), Some("ready"));
        assert!(log.read().iter().any(|(pid, _)| *pid == p.pid()));
        assert!(log.leftovers().is_empty());
        p.write(b"\n");
        read(&rx, &mut String::new(), None).expect("exits");
        assert!(log.read().is_empty());

        // A group an earlier devdeck left: an orphan in its own group.
        let out = std::process::Command::new("sh")
            .args(["-c", "sleep 30 >/dev/null 2>&1 & echo $!"])
            .process_group(0)
            .output()
            .expect("sh runs");
        let orphan: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().expect("pid");
        let start = stat(orphan).expect("orphan runs").start;
        let _reap = Reap(vec![(orphan, start)]);
        // Next to it: a pid that runs with another start time, and one that doesn't run.
        let text = format!("{orphan} {start}\n{} 1\n999999999 5\ngarbage\n", std::process::id());
        crate::state::write_atomic(&log.path, &text).unwrap();
        assert_eq!(log.leftovers(), vec![orphan]);
        kill_all(&log.leftovers());
        wait_until("the leftover to end", || !alive(orphan, start));
        assert!(log.leftovers().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
