use rustix::io::Errno;
use rustix::process::{Pid, test_kill_process};

/// `kill(pid, 0)`: gone only when no process has the pid (`ESRCH`); another user's
/// (`EPERM`) exists.
pub fn exists(pid: u32) -> bool {
    let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else { return false };
    !matches!(test_kill_process(pid), Err(Errno::SRCH))
}
