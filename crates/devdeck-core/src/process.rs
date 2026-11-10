//! Real processes for runs (R6.7, R8.2), each in a pseudo-terminal, in its own session and
//! process group (the pid is the group id).
//!
//! - Output is read on a thread and sent as bytes; the exit is sent once the output is
//!   drained (with a timeout: a grandchild may keep the terminal open), with the code or
//!   the signal number.
//! - Stop: ^C (SIGINT) to the process group, then SIGTERM, then SIGKILL, each after its
//!   [`StopKind`]'s wait; descendants that left the group (found through
//!   `devdeck_sys::procs`) get SIGTERM and SIGKILL too (a terminal's ^C never reaches them
//!   either). It ends once nothing is left. Dropping a [`Process`] kills it at once.
//! - Where it can ([`scope`](crate::scope)), a run starts in a systemd scope of its own,
//!   which keeps every descendant whatever it does: a stop empties it. A run that ends on
//!   its own ends its group; what it left in the scope (a build's daemon) runs on until
//!   devdeck quits ([`GroupLog::end_ours`]).
//! - A terminal's shell gets no scope, and its end reaches only its session: what was
//!   detached on purpose (`setsid`, tmux) stays, as with any terminal.
//! - Every started group and scope is recorded in the state folder ([`GroupLog`]), so a
//!   devdeck that was killed can offer to kill what it left behind at next start.
//! - [`kill_all`] is safe to call from a signal-handling thread (devdeck receiving
//!   SIGTERM, SIGHUP or SIGINT).
//!
//! The terminal is opened here rather than with `alacritty_terminal`'s `tty`, which does
//! the same (`openpty`, `setsid`, `TIOCSCTTY`) but always passes devdeck's own environment
//! on, ignores a missing folder, and blocks in its `Drop` until the child exits. One
//! thread per run polls the terminal, the child's pidfd and a wake-up for input.

use crate::apps::Command;
use crate::runs::Exit;
use crate::scope::Scope;
use crate::terminal::TermSize;
use devdeck_sys::procs::{self, alive};
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
    /// A terminal's shell, as when a terminal closes: SIGHUP to everything (an
    /// interactive shell ignores ^C and SIGTERM and hangs its jobs up), then 2 s,
    /// SIGTERM, then 300 ms, SIGKILL.
    HangUp,
    /// Tests: 300 ms each.
    #[cfg(test)]
    Quick,
}

impl StopKind {
    /// docker compose commands get [`StopKind::Compose`], a terminal's shell
    /// [`StopKind::HangUp`].
    pub fn of(cmd: &Command) -> StopKind {
        match cmd.source() {
            "compose" => StopKind::Compose,
            "shell" => StopKind::HangUp,
            _ => StopKind::Default,
        }
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
            StopKind::HangUp => (Duration::from_secs(2), Duration::from_millis(300)),
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
    /// In a scope of its own, where scopes work here (a terminal's shell: no).
    pub scope: bool,
    /// What it is, for the leftovers a killed devdeck offers (`web · dev`).
    pub label: String,
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
        let scope = spawn.scope.then(crate::scope::systemd_run).flatten().map(|run| (Scope::next(&instance()), run));
        // `systemd-run` reaches the user manager through these; taken from devdeck when the
        // run's environment lacks them, and kept from the run itself.
        let lent: Vec<(&str, std::ffi::OsString)> = match scope {
            Some(_) => ["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"]
                .into_iter()
                .filter(|name| !spawn.vars.iter().any(|(k, _)| k == name))
                .filter_map(|name| Some((name, std::env::var_os(name)?)))
                .collect(),
            None => Vec::new(),
        };
        let mut cmd = match &scope {
            Some((s, run)) => s.command(run, &lent.iter().map(|(n, _)| *n).collect::<Vec<_>>()),
            None => std::process::Command::new("/bin/sh"),
        };
        cmd.arg("-c").arg(&spawn.line).current_dir(&spawn.dir).env_clear().envs(spawn.vars.iter().map(|(k, v)| (k, v))).envs(lent);
        let scope = scope.map(|(s, _)| s);
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
        let start = procs::get(pid).map_or(0, |s| s.start);
        let tree = Arc::new(Tree {
            pid,
            start,
            scope,
            session_only: spawn.stop == StopKind::HangUp,
            stopping: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            seen: Mutex::new(BTreeMap::new()),
        });
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
        let pump = Pump { tree: tree.clone(), label: spawn.label, master: master.clone(), pidfd, input: input.clone(), tx, log };
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

    /// Its start time (clock ticks since boot): with the pid, names this process only.
    pub fn start(&self) -> u64 {
        self.tree.start
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

    /// Something other than the process itself runs in its terminal's foreground (a
    /// shell running a command), as terminal emulators tell before closing.
    pub fn busy(&self) -> bool {
        self.foreground().is_some()
    }

    /// The process group in its terminal's foreground, when it isn't the process's own.
    pub fn foreground(&self) -> Option<u32> {
        let group = rustix::termios::tcgetpgrp(&*self.master).ok()?.as_raw_nonzero().get() as u32;
        (group != self.tree.pid).then_some(group)
    }

    /// The name of the program in its terminal's foreground (`vite`), when it isn't the
    /// process itself.
    pub fn foreground_program(&self) -> Option<String> {
        let group = self.foreground()?;
        procs::name(group)
    }

    /// Starts the stop sequence (returns at once; the exit arrives as an event).
    pub fn stop(&self) {
        if self.tree.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        let tree = self.tree.clone();
        let (waits, hang_up) = (self.stop.waits(), self.stop == StopKind::HangUp);
        let name = format!("stop {}", tree.pid);
        if std::thread::Builder::new().name(name).spawn(move || tree.stop(waits, hang_up)).is_err() {
            self.tree.collect();
            self.tree.signal_all(Signal::KILL);
        }
    }
}

impl Drop for Process {
    /// Kills everything on a thread: finding the processes that left the group reads all
    /// of `/proc` (not for the caller, the UI), and must come before the kill, which
    /// would orphan them out of reach. After the process ended on its own, what it left
    /// in its scope runs on (until devdeck quits).
    fn drop(&mut self) {
        let tree = self.tree.clone();
        let name = format!("drop {}", tree.pid);
        let finish = move || {
            tree.collect();
            let ended = tree.ended.load(Ordering::SeqCst);
            tree.signal(Signal::KILL, !ended);
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
    tree: Arc<Tree>,
    label: String,
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
            log.add(Entry::Group { pid: self.tree.pid, start: self.tree.start });
            if let Some(scope) = &self.tree.scope {
                log.add(Entry::Scope { unit: scope.unit.clone(), instance: instance(), label: self.label.clone() });
            }
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
                self.tree.ended.store(true, Ordering::SeqCst);
                if let Some(log) = &self.log {
                    log.remove(self.tree.pid);
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

/// A run's processes: its group, its scope when it has one, and every descendant seen
/// so far, which may have left the group or lost their parent since.
struct Tree {
    pid: u32,
    /// The leader's start time: a pid reused after it ended isn't ours.
    start: u64,
    scope: Option<Scope>,
    /// A terminal's shell: only what stayed in its session is its own.
    session_only: bool,
    stopping: AtomicBool,
    /// The leader ended on its own (or was stopped) and was reaped.
    ended: AtomicBool,
    /// pid → start time.
    seen: Mutex<BTreeMap<u32, u64>>,
}

impl Tree {
    /// ^C to the group (SIGHUP to everything when `hang_up`), then SIGTERM, then
    /// SIGKILL, each after its wait; ends once nothing is left.
    fn stop(&self, waits: (Duration, Duration), hang_up: bool) {
        self.collect();
        if hang_up {
            self.signal_all(Signal::HUP);
        } else {
            self.signal_group(Signal::INT);
        }
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

    /// `signal` to the group and the descendants seen, and to the scope when `scope`.
    fn signal(&self, signal: Signal, scope: bool) {
        self.signal_group(signal);
        let seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        for (pid, start) in seen.iter() {
            if alive(*pid, *start) {
                signal_one(*pid, signal);
            }
        }
        match &self.scope {
            Some(s) if scope && signal == Signal::KILL => s.kill(),
            Some(s) if scope => s.signal(signal),
            _ => {}
        }
    }

    /// Adds the live processes below the leader and below those seen before (an orphan's
    /// children are still below it), and forgets those that ended.
    fn collect(&self) {
        let pairs = procs::parents().unwrap_or_default();
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|pid, start| alive(*pid, *start));
        let mut roots: Vec<u32> = seen.keys().copied().collect();
        if alive(self.pid, self.start) {
            roots.push(self.pid);
        }
        for root in roots {
            for pid in procs::tree(root, &pairs) {
                if pid != self.pid && !seen.contains_key(&pid)
                    && let Some(s) = procs::get(pid)
                    && (!self.session_only || s.session == self.pid)
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
        procs::get(self.pid).is_none_or(|s| s.start == self.start)
    }

    fn signal_group(&self, signal: Signal) {
        if self.group_is_ours() {
            signal_group(self.pid, signal);
        }
    }

    /// The group, the descendants seen and the scope.
    fn signal_all(&self, signal: Signal) {
        self.signal(signal, true);
    }

    fn gone(&self) -> bool {
        let group = self.group_is_ours() && Pid::from_raw(self.pid as i32).is_some_and(|p| rustix::process::test_kill_process_group(p).is_ok());
        !group && self.seen.lock().unwrap_or_else(|e| e.into_inner()).iter().all(|(pid, start)| !alive(*pid, *start)) && self.scope.as_ref().is_none_or(Scope::is_empty)
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

/// This devdeck: its pid and start time (`1234-5678`), so the scopes of a devdeck that's
/// gone are told from those of one that runs.
fn instance() -> String {
    let pid = std::process::id();
    format!("{pid}-{}", procs::get(pid).map_or(0, |s| s.start))
}

/// `instance` (as [`instance`] writes it) still runs.
fn instance_alive(instance: &str) -> bool {
    instance.split_once('-').and_then(|(p, s)| Some((p.parse().ok()?, s.parse().ok()?))).is_some_and(|(pid, start)| alive(pid, start))
}

/// What devdeck started and may still run, in a file of the state folder: each run's
/// group (`pid start` lines, while its leader runs) and each run's scope (`scope <unit>
/// <instance> <label>` lines, while its devdeck runs or it holds processes).
#[derive(Clone, Debug)]
pub struct GroupLog {
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Entry {
    Group { pid: u32, start: u64 },
    Scope { unit: String, instance: String, label: String },
}

impl Entry {
    fn parse(line: &str) -> Option<Entry> {
        if let Some(rest) = line.strip_prefix("scope ") {
            let mut parts = rest.splitn(3, ' ');
            let (unit, instance) = (parts.next()?.to_string(), parts.next()?.to_string());
            return Some(Entry::Scope { unit, instance, label: parts.next().unwrap_or_default().to_string() });
        }
        let (pid, start) = line.split_once(' ')?;
        Some(Entry::Group { pid: pid.parse().ok()?, start: start.parse().ok()? })
    }

    fn line(&self) -> String {
        match self {
            Entry::Group { pid, start } => format!("{pid} {start}\n"),
            Entry::Scope { unit, instance, label } => format!("scope {unit} {instance} {label}\n"),
        }
    }

    /// Still worth keeping: a group whose leader runs; a scope whose devdeck runs (it may
    /// not hold anything yet) or that holds processes.
    fn live(&self) -> bool {
        match self {
            Entry::Group { pid, start } => alive(*pid, *start),
            Entry::Scope { unit, instance, .. } => instance_alive(instance) || !Scope::named(unit.clone()).is_empty(),
        }
    }
}

/// What a devdeck that's gone left running: a run's group whose leader still runs, or a
/// run's scope that still holds processes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leftover {
    /// The run's name for a scope (`web · dev`); none for a group (its command says).
    pub label: Option<String>,
    /// What runs in it now; a group's leader alone.
    pub pids: Vec<u32>,
    entry: Entry,
}

/// Several runs start and end at once: the file is read and rewritten under this lock.
static LOG_LOCK: Mutex<()> = Mutex::new(());

impl GroupLog {
    pub fn new(state_dir: &Path) -> GroupLog {
        GroupLog { path: state_dir.join("processes") }
    }

    /// What an earlier devdeck left running: groups (same pid and start time, not our
    /// children) and scopes of a devdeck that's gone, still holding processes. Reads
    /// `/proc` and the cgroup tree: not on the UI thread.
    pub fn leftovers(&self) -> Vec<Leftover> {
        let me = std::process::id();
        let entries = self.read();
        let scopes: Vec<Leftover> = entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Scope { unit, instance, label } if !instance_alive(instance) => {
                    let pids = Scope::named(unit.clone()).pids();
                    (!pids.is_empty()).then(|| Leftover { label: Some(label.clone()), pids, entry: entry.clone() })
                }
                _ => None,
            })
            .collect();
        // A run's leader still running is in its scope's row already.
        let groups = entries.iter().filter_map(|entry| match entry {
            Entry::Group { pid, start } if alive(*pid, *start) && procs::parent_of(*pid) != Some(me) && !scopes.iter().any(|s| s.pids.contains(pid)) => {
                Some(Leftover { label: None, pids: vec![*pid], entry: entry.clone() })
            }
            _ => None,
        });
        groups.chain(scopes.iter().cloned()).collect()
    }

    /// Every recorded group still running, ours included: what to kill when devdeck
    /// itself is told to quit (a signal-handling thread can't reach the runner).
    pub fn live(&self) -> Vec<u32> {
        self.read()
            .into_iter()
            .filter_map(|e| match e {
                Entry::Group { pid, start } if alive(pid, start) => Some(pid),
                _ => None,
            })
            .collect()
    }

    /// Quitting: kills what this devdeck's runs left in their scopes (a build's daemon
    /// runs on after its run, until devdeck quits) and forgets them.
    pub fn end_ours(&self) {
        let me = instance();
        let ours: Vec<Entry> = self.read().into_iter().filter(|e| matches!(e, Entry::Scope { instance, .. } if *instance == me)).collect();
        for e in &ours {
            if let Entry::Scope { unit, .. } = e {
                Scope::named(unit.clone()).kill();
            }
        }
        self.update(|entries| entries.retain(|e| !ours.contains(e)));
    }

    fn read(&self) -> Vec<Entry> {
        std::fs::read_to_string(&self.path).unwrap_or_default().lines().filter_map(Entry::parse).collect()
    }

    /// Rewrites the file with `change` applied, dropping entries that ended. Best effort:
    /// a run starts even when it can't be recorded.
    fn update(&self, change: impl FnOnce(&mut Vec<Entry>)) {
        let _lock = LOG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut entries = self.read();
        entries.retain(Entry::live);
        change(&mut entries);
        let text: String = entries.iter().map(Entry::line).collect();
        let _ = crate::state::write_atomic(&self.path, &text);
    }

    /// Kills leftovers the user chose to kill (a group only while it's still the process
    /// offered, same start time; a scope whole), and forgets them.
    pub fn kill(&self, leftovers: &[Leftover]) {
        for l in leftovers {
            match &l.entry {
                Entry::Group { pid, start } if alive(*pid, *start) => kill_all(&[*pid]),
                Entry::Scope { unit, .. } => Scope::named(unit.clone()).kill(),
                Entry::Group { .. } => {}
            }
        }
        self.forget(leftovers);
    }

    /// Stops tracking these: leftovers the user chose to leave aren't killed when this
    /// devdeck quits, nor offered again.
    pub fn forget(&self, leftovers: &[Leftover]) {
        self.update(|entries| entries.retain(|e| !leftovers.iter().any(|l| l.entry == *e)));
    }

    fn add(&self, entry: Entry) {
        self.update(|entries| entries.push(entry));
    }

    fn remove(&self, pid: u32) {
        self.update(|entries| entries.retain(|e| !matches!(e, Entry::Group { pid: p, .. } if *p == pid)));
    }
}

/// Asks one process to end (SIGTERM): a port holder the user chose to stop. Never pid 0
/// or 1, nor devdeck itself.
pub fn terminate(pid: u32) -> Result<(), String> {
    let p = ours(pid).ok_or_else(|| format!("won't stop pid {pid}"))?;
    rustix::process::kill_process(p, Signal::TERM).map_err(|e| format!("couldn't stop pid {pid}: {e}"))
}

/// Kills these process groups and their escaped descendants (SIGKILL), now. Reads the
/// process table and allocates: call it from a thread (signal-hook's iterator), not a
/// signal handler.
pub fn kill_all(pids: &[u32]) {
    // Descendants first: once a leader is killed, its children move to another parent.
    let pairs = procs::parents().unwrap_or_default();
    let all: BTreeSet<u32> = pids.iter().filter(|p| ours(**p).is_some()).flat_map(|p| procs::tree(*p, &pairs)).collect();
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
        let spawn = Spawn { line: line.into(), dir: std::env::temp_dir(), vars, size: SIZE, stop: StopKind::Quick, scope: false, label: String::new() };
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
        procs::get(pid).map(|p| p.session)
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

    /// Spawns in a scope; `None` (the test is skipped) where scopes don't work.
    fn spawn_scoped(line: &str, log: Option<GroupLog>) -> Option<(Process, Receiver<ProcessEvent>)> {
        if crate::scope::systemd_run().is_none() {
            eprintln!("no systemd user manager here: scope test skipped");
            return None;
        }
        let vars = vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())];
        let spawn = Spawn { line: line.into(), dir: std::env::temp_dir(), vars, size: SIZE, stop: StopKind::Quick, scope: true, label: "web · dev".into() };
        Some(Process::spawn(spawn, log).expect("spawns"))
    }

    /// A running process: its pid and start time.
    fn running(pid: u32) -> (u32, u64) {
        (pid, procs::get(pid).expect("runs").start)
    }

    #[test]
    fn a_stop_empties_the_scope_daemons_and_cleared_environments_included() {
        // The subshell's exit orphans the daemon at once: it's out of the run's tree.
        let line = "( setsid sleep 300 & echo daemon $! ); env -i /bin/sleep 300 & echo bare $!; echo ready; sleep 300";
        let Some((p, rx)) = spawn_scoped(line, None) else { return };
        let mut out = String::new();
        read(&rx, &mut out, Some("ready"));
        let (daemon, bare) = (running(pid_after(&out, "daemon ")), running(pid_after(&out, "bare ")));
        let _reap = Reap(vec![daemon, bare]);
        wait_until("the daemon to leave the tree", || !procs::tree(p.pid(), &procs::parents().unwrap()).contains(&daemon.0));
        p.stop();
        assert!(read(&rx, &mut out, None).is_some(), "ends");
        wait_until("the daemon and the bare one to end", || !alive(daemon.0, daemon.1) && !alive(bare.0, bare.1));
    }

    #[test]
    fn a_run_that_ends_on_its_own_leaves_its_daemon_until_devdeck_quits() {
        let dir = crate::testing::TempDir::new("scope-quit");
        let log = GroupLog::new(&dir);
        // The pause lets `setsid` detach before the run's end hangs its terminal up.
        let Some((p, rx)) = spawn_scoped("( setsid sleep 300 & echo daemon $!; sleep 0.3 ); echo done", Some(log.clone())) else { return };
        let mut out = String::new();
        assert_eq!(read(&rx, &mut out, None), Some(Exit::Code(0)));
        let daemon = running(pid_after(&out, "daemon "));
        let _reap = Reap(vec![daemon]);
        drop(p);
        std::thread::sleep(Duration::from_millis(300));
        assert!(alive(daemon.0, daemon.1), "a build's daemon runs on after its run");
        log.end_ours();
        wait_until("the daemon to end as devdeck quits", || !alive(daemon.0, daemon.1));
        assert!(log.read().is_empty(), "forgotten");
    }

    #[test]
    fn a_scope_a_killed_devdeck_left_is_offered_and_killed() {
        let dir = crate::testing::TempDir::new("scope-leftover");
        let log = GroupLog::new(&dir);
        let Some((p, rx)) = spawn_scoped("( setsid sleep 300 & echo daemon $! ); echo ready; sleep 300", Some(log.clone())) else { return };
        let mut out = String::new();
        read(&rx, &mut out, Some("ready"));
        let daemon = running(pid_after(&out, "daemon "));
        let _reap = Reap(vec![daemon]);
        wait_until("the scope to be recorded", || log.read().iter().any(|e| matches!(e, Entry::Scope { .. })));
        // As if the devdeck that started it were gone.
        let gone: String = log
            .read()
            .into_iter()
            .map(|e| match e {
                Entry::Scope { unit, label, .. } => Entry::Scope { unit, instance: "999999999-1".into(), label },
                group => group,
            })
            .map(|e| e.line())
            .collect();
        crate::state::write_atomic(&log.path, &gone).unwrap();
        let left = log.leftovers();
        assert_eq!(left.len(), 1, "the scope, not the group (our own child): {left:?}");
        assert_eq!(left[0].label.as_deref(), Some("web · dev"));
        assert!(left[0].pids.contains(&daemon.0) && left[0].pids.contains(&p.pid()), "{left:?}");
        log.kill(&left);
        wait_until("the scope to be emptied", || !alive(daemon.0, daemon.1));
        assert!(read(&rx, &mut out, None).is_some(), "the run ended with it");
        assert!(log.leftovers().is_empty());
    }

    #[test]
    fn a_leader_that_outlived_a_killed_devdeck_is_in_its_scopes_row_only() {
        if crate::scope::systemd_run().is_none() {
            eprintln!("no systemd user manager here: scope test skipped");
            return;
        }
        let dir = crate::testing::TempDir::new("scope-one-row");
        let log = GroupLog::new(&dir);
        // A run's leader that survived the hang-up, in its scope, not our child.
        let unit = format!("devdeck-test-{}-one-row", std::process::id());
        let line = format!("systemd-run --user --scope --quiet --collect --unit={unit} sleep 300 >/dev/null 2>&1 & echo $!");
        let out = std::process::Command::new("sh").args(["-c", &line]).output().expect("sh runs");
        let leader = running(String::from_utf8_lossy(&out.stdout).trim().parse().expect("pid"));
        let _reap = Reap(vec![leader]);
        wait_until("the scope", || !Scope::named(unit.clone()).is_empty());
        let text = format!("{} {}\nscope {unit} 999999999-1 web · dev\n", leader.0, leader.1);
        crate::state::write_atomic(&log.path, &text).unwrap();
        let left = log.leftovers();
        assert_eq!(left.len(), 1, "one row for the run: {left:?}");
        assert_eq!((left[0].label.as_deref(), left[0].pids.clone()), (Some("web · dev"), vec![leader.0]));
        log.kill(&left);
        wait_until("the leader to end", || !alive(leader.0, leader.1));
    }

    #[test]
    fn a_terminals_end_leaves_what_left_its_session() {
        let spawn = Spawn {
            line: "setsid sleep 300 & echo left $!; sleep 300 & echo job $!; echo ready; wait".into(),
            dir: std::env::temp_dir(),
            vars: vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())],
            size: SIZE,
            stop: StopKind::HangUp,
            scope: false,
            label: String::new(),
        };
        let (p, rx) = Process::spawn(spawn, None).expect("spawns");
        let mut out = String::new();
        read(&rx, &mut out, Some("ready"));
        let (left, job) = (running(pid_after(&out, "left ")), running(pid_after(&out, "job ")));
        let _reap = Reap(vec![left, job]);
        wait_until("`setsid` to take its own session", || session_of(left.0) == Some(left.0));
        p.stop();
        assert!(read(&rx, &mut out, None).is_some(), "ends");
        wait_until("the job to end", || !alive(job.0, job.1));
        assert!(alive(left.0, left.1), "what left the session stays, as with any terminal");
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
        let spawn = Spawn { line: "true".into(), dir: dir.clone(), vars: vec![], size: SIZE, stop: StopKind::Quick, scope: false, label: String::new() };
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

    #[test]
    fn busy_while_a_job_holds_the_foreground() {
        // An interactive shell gives the terminal to its foreground job, a job its own
        // group: here `sh -m` (job control) running `sleep` in the foreground.
        let (p, rx) = spawn("exec sh -m -c 'echo ready; sleep 0.5; echo back; sleep 30'");
        let mut out = String::new();
        read(&rx, &mut out, Some("ready"));
        wait_until("the job in the foreground", || p.busy());
        assert_eq!(p.foreground_program().as_deref(), Some("sleep"));
        read(&rx, &mut out, Some("back"));
        p.stop();
        // Without job control, what runs stays in the process's own group: not busy.
        let (q, rx) = spawn("echo ready; sleep 30");
        let mut out = String::new();
        read(&rx, &mut out, Some("ready"));
        assert!(!q.busy());
    }

    #[test]
    fn a_hang_up_ends_what_ignores_ctrl_c_and_sigterm_at_once() {
        // As an interactive shell does.
        let spawn = Spawn {
            line: "trap '' INT TERM; echo ready; sleep 30".into(),
            dir: std::env::temp_dir(),
            vars: vec![("PATH".into(), std::env::var("PATH").unwrap_or_default())],
            size: SIZE,
            stop: StopKind::HangUp,
            scope: false,
            label: String::new(),
        };
        let (p, rx) = Process::spawn(spawn, None).expect("spawns");
        let mut out = String::new();
        read(&rx, &mut out, Some("ready"));
        let at = Instant::now();
        p.stop();
        assert_eq!(read(&rx, &mut out, None), Some(Exit::Signal(1)), "SIGHUP");
        assert!(at.elapsed() < Duration::from_secs(1), "no wait for SIGTERM or SIGKILL: {:?}", at.elapsed());
    }

    /// A child in its own session, as the run prints it: `(process, its pid, its start)`.
    fn with_escaped_child() -> (Process, Receiver<ProcessEvent>, u32, u64) {
        let (p, rx) = spawn("setsid sleep 30 & echo \"child $!\"; wait");
        let mut out = String::new();
        read(&rx, &mut out, Some("child "));
        read(&rx, &mut out, Some("\n"));
        let child = pid_after(&out, "child ");
        let start = procs::get(child).expect("child runs").start;
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
        let start = procs::get(child).expect("child runs").start;
        let _reap = Reap(vec![(child, start)]);
        let at = Instant::now();
        assert_eq!(read(&rx, &mut out, None), Some(Exit::Code(0)));
        assert!(at.elapsed() >= DRAIN - Duration::from_millis(200), "{:?}", at.elapsed());
        // Still in the group after its leader ended: dropping kills it.
        drop(p);
        wait_until("the grandchild to end", || !alive(child, start));
    }

    #[test]
    fn the_group_log_keeps_runs_until_they_exit_and_finds_leftovers() {
        let dir = crate::testing::TempDir::new("group-log");
        let log = GroupLog::new(&dir);

        // A run is recorded while it runs, isn't a leftover of ours, and is removed at exit.
        let spawn = Spawn { line: "echo ready; read x".into(), dir: dir.parent().unwrap().into(), vars: vec![], size: SIZE, stop: StopKind::Quick, scope: false, label: String::new() };
        let (p, rx) = Process::spawn(spawn, Some(log.clone())).expect("spawns");
        read(&rx, &mut String::new(), Some("ready"));
        assert!(log.read().contains(&Entry::Group { pid: p.pid(), start: procs::get(p.pid()).unwrap().start }));
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
        let start = procs::get(orphan).expect("orphan runs").start;
        let _reap = Reap(vec![(orphan, start)]);
        // Next to it: a pid that runs with another start time, and one that doesn't run.
        let text = format!("{orphan} {start}\n{} 1\n999999999 5\ngarbage\n", std::process::id());
        crate::state::write_atomic(&log.path, &text).unwrap();
        let left = log.leftovers();
        assert_eq!(left.iter().map(|l| (l.label.clone(), l.pids.clone())).collect::<Vec<_>>(), [(None, vec![orphan])]);
        log.kill(&left);
        wait_until("the leftover to end", || !alive(orphan, start));
        assert!(log.leftovers().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
