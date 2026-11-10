//! Sockets (U11): ending a listener's wait, and seeing a client hang up.

use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;

mod unix;

/// Shuts a socket down both ways: a thread blocked accepting on it wakes.
pub fn shutdown(fd: &OwnedFd) {
    unix::shutdown(fd)
}

/// Whether the client closed its end: a read would end. Looked at without blocking or
/// taking anything.
pub fn hung_up(stream: &UnixStream) -> bool {
    unix::hung_up(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::os::unix::net::UnixListener;
    use std::time::Duration;

    #[test]
    fn a_closed_end_is_seen() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        assert!(!hung_up(&ours));
        drop(theirs);
        assert!(hung_up(&ours));
    }

    #[test]
    fn data_waiting_is_not_a_hang_up() {
        use std::io::Write;
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        theirs.write_all(b"x").unwrap();
        assert!(!hung_up(&ours));
    }

    #[test]
    fn shutdown_wakes_a_blocked_accept() {
        let dir = TempDir::new("net-accept");
        let listener = UnixListener::bind(dir.join("s.sock")).unwrap();
        let fd: OwnedFd = listener.try_clone().unwrap().into();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(listener.accept().is_ok());
        });
        std::thread::sleep(Duration::from_millis(100));
        shutdown(&fd);
        let accepted = rx.recv_timeout(Duration::from_secs(1)).expect("accept wakes within 1 s");
        assert!(!accepted, "nothing connected");
    }
}
