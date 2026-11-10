use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

pub fn shutdown(fd: &OwnedFd) {
    let _ = rustix::net::shutdown(fd, rustix::net::Shutdown::Both);
}

/// POSIX `recv` with `MSG_PEEK | MSG_DONTWAIT` (std's `peek` isn't stable).
pub fn hung_up(stream: &UnixStream) -> bool {
    let mut byte = 0u8;
    // SAFETY: a valid descriptor, one byte of our own buffer.
    let n = unsafe { libc::recv(stream.as_raw_fd(), (&mut byte as *mut u8).cast(), 1, libc::MSG_PEEK | libc::MSG_DONTWAIT) };
    match n {
        0 => true,
        n if n > 0 => false,
        _ => {
            let e = std::io::Error::last_os_error().kind();
            e != std::io::ErrorKind::WouldBlock && e != std::io::ErrorKind::Interrupted
        }
    }
}
