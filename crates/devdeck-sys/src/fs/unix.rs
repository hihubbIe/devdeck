use std::fs::{File, Metadata, OpenOptions};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

pub fn try_lock(file: &File) -> std::io::Result<()> {
    rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(Into::into)
}

pub fn no_follow(options: &mut OpenOptions, nonblocking: bool) -> &mut OpenOptions {
    options.custom_flags(libc::O_NOFOLLOW | if nonblocking { libc::O_NONBLOCK } else { 0 })
}

pub fn is_mine(meta: &Metadata) -> bool {
    meta.uid() == rustix::process::getuid().as_raw()
}

pub fn is_loop(e: &std::io::Error) -> bool {
    rustix::io::Errno::from_io_error(e) == Some(rustix::io::Errno::LOOP)
}

pub fn make_fifo(path: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    // SAFETY: a NUL-terminated path we own.
    if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}
