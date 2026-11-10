//! `SO_PEERCRED` for the user and the pid, then `SO_PEERPIDFD` (Linux 6.5) for a pidfd
//! of the very process; its pid comes from the pidfd's `fdinfo`, and must be the one the
//! credentials gave.

use super::Refused;
use crate::Support;
use rustix::event::{PollFd, PollFlags, Timespec};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

pub const SUPPORT: Support = Support::Yes;

pub type Pin = OwnedFd;

pub fn of(stream: &UnixStream) -> Result<Option<(u32, Pin)>, Refused> {
    let cred = rustix::net::sockopt::socket_peercred(stream).map_err(|e| Refused::NoCredentials(e.into()))?;
    // Another user's process is never looked at: refused before its pidfd is asked for.
    same_user(cred.uid.as_raw(), rustix::process::getuid().as_raw())?;
    let Some(pidfd) = peer_pidfd(stream) else { return Ok(None) };
    Ok(pinned(cred.pid.as_raw_nonzero().get() as u32, pidfd_pid(&pidfd)).map(|pid| (pid, pidfd)))
}

fn same_user(peer_uid: u32, own_uid: u32) -> Result<(), Refused> {
    if peer_uid == own_uid { Ok(()) } else { Err(Refused::OtherUser(peer_uid)) }
}

/// The pid when the pidfd names the process the credentials gave; None (not pinned) when
/// it's gone or another one.
fn pinned(cred_pid: u32, pidfd_pid: Option<u32>) -> Option<u32> {
    pidfd_pid.filter(|p| *p == cred_pid)
}

pub fn ended(pin: &Pin) -> bool {
    let mut fds = [PollFd::new(pin, PollFlags::IN)];
    rustix::event::poll(&mut fds, Some(&Timespec { tv_sec: 0, tv_nsec: 0 })).map_or(true, |n| n > 0)
}

fn peer_pidfd(stream: &UnixStream) -> Option<OwnedFd> {
    let mut fd: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: a connected socket, and an int with its size, which SO_PEERPIDFD fills.
    let r = unsafe { libc::getsockopt(stream.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERPIDFD, (&raw mut fd).cast(), &mut len) };
    if r != 0 || fd < 0 || len as usize != std::mem::size_of::<libc::c_int>() {
        return None;
    }
    // SAFETY: the kernel just made this descriptor for us; nothing else owns it.
    Some(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The pid a pidfd stands for (its `fdinfo`); None once the process is gone (-1).
fn pidfd_pid(fd: &OwnedFd) -> Option<u32> {
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", fd.as_raw_fd())).ok()?;
    info.lines().find_map(|l| l.strip_prefix("Pid:"))?.trim().parse::<i64>().ok().filter(|p| *p > 0).map(|p| p as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_user_is_refused() {
        assert!(matches!(same_user(1001, 1000), Err(Refused::OtherUser(1001))));
        assert!(same_user(1000, 1000).is_ok());
    }

    #[test]
    fn a_pidfd_for_another_process_or_a_gone_one_is_not_pinned() {
        assert_eq!(pinned(42, Some(42)), Some(42));
        assert_eq!(pinned(42, Some(43)), None, "the pid was used again");
        assert_eq!(pinned(42, None), None, "the process is gone");
    }
}
