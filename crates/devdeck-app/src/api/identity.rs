//! Who connects (R13.4), found from processes, never from what the caller says.
//!
//! The kernel gives the peer's user and a pidfd (`SO_PEERPIDFD`), which pins the very
//! process that connected: a pid used again can't pass for it. From it the chain of its
//! ancestors is read from `/proc`, each started no later than its child (otherwise the
//! pid in between was used again, and the chain stops there). The UI thread then matches
//! the chain against the shells devdeck runs ([`super::dispatch`]). Anything failing
//! here makes an outside caller, never a terminal.

use rustix::event::{PollFd, PollFlags, Timespec};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

/// `SO_PEERPIDFD` (Linux 6.5; the generic value, which x86-64 and arm64 use).
const SO_PEERPIDFD: libc::c_int = 77;

/// Ancestors read at most (a deeper chain is cut: its top can't be a devdeck shell).
const MAX_CHAIN: usize = 64;

/// A connected process, as far as it could be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// None: it couldn't be pinned (it's then outside, whatever its ancestors).
    pub pid: Option<u32>,
    /// The process, then its parent, and so on, each with its start time.
    pub chain: Vec<(u32, u64)>,
    /// Its name, untrusted.
    pub program: String,
}

impl Peer {
    fn unpinned() -> Self {
        Self { pid: None, chain: Vec::new(), program: "?".into() }
    }
}

/// The peer of `stream`; an error when it's another user (the connection is refused).
pub fn peer(stream: &UnixStream) -> Result<Peer, String> {
    let cred = rustix::net::sockopt::socket_peercred(stream).map_err(|e| format!("no peer credentials: {e}"))?;
    if cred.uid != rustix::process::getuid() {
        return Err(format!("another user (uid {})", cred.uid.as_raw()));
    }
    let Some(pidfd) = peer_pidfd(stream) else { return Ok(Peer::unpinned()) };
    let Some(pid) = pidfd_pid(&pidfd).filter(|p| *p == cred.pid.as_raw_nonzero().get() as u32) else { return Ok(Peer::unpinned()) };
    let chain = chain(pid, devdeck_core::procfs::parent_and_start);
    let program = devdeck_core::procfs::name(pid).unwrap_or_else(|| "?".into());
    // Read while it lived: what was read is its own.
    if exited(&pidfd) || chain.is_empty() {
        return Ok(Peer::unpinned());
    }
    Ok(Peer { pid: Some(pid), chain, program })
}

/// `pid` and its ancestors with their start times, from `stat` (`(parent, start)`),
/// while each parent started no later than its child.
pub fn chain(pid: u32, stat: impl Fn(u32) -> Option<(u32, u64)>) -> Vec<(u32, u64)> {
    let mut out = Vec::new();
    let Some((mut parent, mut start)) = stat(pid) else { return out };
    out.push((pid, start));
    while parent > 1 && out.len() < MAX_CHAIN {
        let Some((grand, parent_start)) = stat(parent) else { break };
        if parent_start > start {
            break;
        }
        out.push((parent, parent_start));
        (parent, start) = (grand, parent_start);
    }
    out
}

fn peer_pidfd(stream: &UnixStream) -> Option<OwnedFd> {
    let mut fd: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: a connected socket, and an int with its size, which SO_PEERPIDFD fills.
    let r = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, SO_PEERPIDFD, (&raw mut fd).cast(), &mut len) };
    if r != 0 || fd < 0 || len as usize != std::mem::size_of::<libc::c_int>() {
        return None;
    }
    // SAFETY: the kernel just made this descriptor for us; nothing else owns it.
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The pid a pidfd stands for (its `fdinfo`); None once the process is gone.
fn pidfd_pid(fd: &OwnedFd) -> Option<u32> {
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", fd.as_raw_fd())).ok()?;
    info.lines().find_map(|l| l.strip_prefix("Pid:"))?.trim().parse::<i64>().ok().filter(|p| *p > 0).map(|p| p as u32)
}

/// Whether the process ended (its pidfd reads as ready).
fn exited(fd: &OwnedFd) -> bool {
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    rustix::event::poll(&mut fds, Some(&Timespec { tv_sec: 0, tv_nsec: 0 })).map_or(true, |n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn procs(list: &[(u32, u32, u64)]) -> impl Fn(u32) -> Option<(u32, u64)> {
        let m: HashMap<u32, (u32, u64)> = list.iter().map(|(p, pp, s)| (*p, (*pp, *s))).collect();
        move |p| m.get(&p).copied()
    }

    #[test]
    fn a_chain_goes_up_to_init() {
        let stat = procs(&[(40, 30, 400), (30, 20, 300), (20, 1, 200)]);
        assert_eq!(chain(40, stat), vec![(40, 400), (30, 300), (20, 200)]);
    }

    #[test]
    fn a_chain_stops_at_a_pid_used_again() {
        // 30 died and a later process took its number: it started after 40.
        let stat = procs(&[(40, 30, 400), (30, 20, 500), (20, 1, 200)]);
        assert_eq!(chain(40, stat), vec![(40, 400)]);
        assert_eq!(chain(99, procs(&[])), Vec::<(u32, u64)>::new(), "a process gone");
    }

    #[test]
    fn this_process_is_pinned_through_a_real_socket() {
        let (a, b) = UnixStream::pair().unwrap();
        let p = peer(&a).unwrap();
        assert_eq!(p.pid, Some(std::process::id()));
        assert_eq!(p.chain.first().map(|c| c.0), Some(std::process::id()));
        assert!(p.chain.len() >= 2);
        drop(b);
    }
}
