//! Listening ports (L4, R6.9): which ports a run's processes listen on, and who holds a
//! port before a start.

use crate::{SysError, Support};

#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(target_os = "linux"), path = "unsupported.rs")]
mod imp;

pub const SUPPORT: Support = imp::SUPPORT;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Listening {
    pub port: u16,
    pub pid: u32,
}

/// Who holds a port.
#[derive(Clone, Debug, PartialEq)]
pub enum Holder {
    /// A process we can see: its pid and command line (its name when that's empty).
    Process { pid: u32, command: String },
    /// Listening, but the process isn't readable (another user).
    Unknown,
}

/// The ports `pids` listen on (TCP, IPv4 and IPv6). Only those pids are looked at.
pub fn listening(pids: &[u32]) -> Result<Vec<Listening>, SysError> {
    imp::listening(pids)
}

/// Who listens on `port` now, if anyone. Looks through every readable process (on
/// demand before a start, not per frame). When several share the socket (a server's
/// workers inherit it), the topmost one is named: the one to stop.
pub fn holder(port: u16) -> Result<Option<Holder>, SysError> {
    imp::holder(port)
}

#[cfg(all(test, not(target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn says_why_there_are_no_ports() {
        assert!(matches!(listening(&[std::process::id()]), Err(SysError::Unsupported(_))));
        assert!(matches!(holder(1), Err(SysError::Unsupported(_))));
    }
}

/// Against real sockets, on an OS with a backend (macOS from step 2d).
#[cfg(all(test, target_os = "linux"))]
mod conformance {
    use super::*;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    fn found_as_this_process(l: &TcpListener) {
        let port = l.local_addr().unwrap().port();
        let me = std::process::id();
        assert!(listening(&[me]).unwrap().contains(&Listening { port, pid: me }), "port {port} found for this process");
        match holder(port).unwrap() {
            Some(Holder::Process { pid, command }) => {
                assert_eq!(pid, me);
                assert!(command.contains("devdeck_sys"), "command: {command}");
            }
            other => panic!("holder of {port}: {other:?}"),
        }
    }

    #[test]
    fn a_real_ipv4_listener_is_found() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        found_as_this_process(&l);
        let port = l.local_addr().unwrap().port();
        drop(l);
        // A process another test spawns meanwhile holds a copy of the listener's descriptor
        // from its fork until its exec closes it (close-on-exec): free within moments.
        // Never this process: it closed its own.
        let deadline = Instant::now() + Duration::from_secs(2);
        while let Some(h) = holder(port).unwrap() {
            assert!(!matches!(h, Holder::Process { pid, .. } if pid == std::process::id()), "still ours once closed");
            assert!(Instant::now() < deadline, "nobody listens once closed");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_real_ipv6_listener_is_found_when_ipv6_is_available() {
        let Ok(l) = TcpListener::bind("[::1]:0") else { return };
        found_as_this_process(&l);
    }

    #[test]
    fn other_pids_are_not_scanned_for_our_ports() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let parent = crate::procs::parent_of(std::process::id()).unwrap();
        assert!(!listening(&[parent]).unwrap().iter().any(|x| x.port == port));
        assert_eq!(listening(&[]).unwrap(), []);
    }
}
