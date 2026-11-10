//! Until macOS step 2e: the user is checked (`getpeereid`), the process isn't pinned, so
//! every caller is an outside one, as on a Linux kernel without `SO_PEERPIDFD`.

use super::Refused;
use crate::Support;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

pub const SUPPORT: Support = Support::Partial("callers are checked by user only on this OS yet");

/// Nothing to hold: nothing is pinned.
pub enum Pin {}

pub fn of(stream: &UnixStream) -> Result<Option<(u32, Pin)>, Refused> {
    let (mut uid, mut gid): (libc::uid_t, libc::gid_t) = (0, 0);
    // SAFETY: a connected socket and two ids of our own, which getpeereid fills.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(Refused::NoCredentials(std::io::Error::last_os_error()));
    }
    if uid != rustix::process::getuid().as_raw() {
        return Err(Refused::OtherUser(uid));
    }
    Ok(None)
}

pub fn ended(pin: &Pin) -> bool {
    match *pin {}
}
