//! Who is on the other end of a Unix socket (L5, R13.4): its user, checked, then the
//! very process, pinned so that a pid used again can't pass for it.

use crate::Support;
use std::fmt;
use std::os::unix::net::UnixStream;

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), path = "unsupported.rs")]
mod imp;

pub const SUPPORT: Support = imp::SUPPORT;

/// Why a peer is refused.
#[derive(Debug)]
pub enum Refused {
    /// Another user: the connection is closed.
    OtherUser(u32),
    /// The OS wouldn't say who it is.
    NoCredentials(std::io::Error),
    /// This OS can't check a peer.
    Unsupported(&'static str),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refused::OtherUser(uid) => write!(f, "another user (uid {uid})"),
            Refused::NoCredentials(e) => write!(f, "no peer credentials: {e}"),
            Refused::Unsupported(why) => f.write_str(why),
        }
    }
}

/// A peer pinned while this is held: its pid can't be used again by another process
/// meanwhile, so what is read about it while held is its own.
pub struct Pinned {
    pub pid: u32,
    pin: imp::Pin,
}

impl Pinned {
    /// Whether the process ended since it connected (asked last, after reading about it);
    /// true when the OS can't tell.
    pub fn ended(&self) -> bool {
        imp::ended(&self.pin)
    }
}

/// The peer of `stream`: refused (another user), not pinned (the OS can't name the very
/// process: it's then an outside caller), or pinned.
pub fn of(stream: &UnixStream) -> Result<Option<Pinned>, Refused> {
    imp::of(stream).map(|p| p.map(|(pid, pin)| Pinned { pid, pin }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_say_why() {
        assert_eq!(Refused::OtherUser(1001).to_string(), "another user (uid 1001)");
        let e = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(Refused::NoCredentials(e).to_string(), "no peer credentials: permission denied");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_socket_to_ourselves_is_pinned_to_this_process() {
        let (a, _b) = UnixStream::pair().unwrap();
        let p = of(&a).unwrap().expect("pinned (needs Linux 6.5 or later for SO_PEERPIDFD)");
        assert_eq!(p.pid, std::process::id());
        assert!(!p.ended());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_socket_to_ourselves_is_our_user_but_not_pinned_yet() {
        let (a, _b) = UnixStream::pair().unwrap();
        assert!(matches!(of(&a), Ok(None)));
    }
}
