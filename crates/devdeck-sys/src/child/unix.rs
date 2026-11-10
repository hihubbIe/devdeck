use super::Signal;
use rustix::process::{Pid, getpgrp, getpid};
use std::os::unix::process::CommandExt;
use std::process::Command;

pub(crate) fn to_rustix(signal: Signal) -> rustix::process::Signal {
    match signal {
        Signal::Hup => rustix::process::Signal::HUP,
        Signal::Int => rustix::process::Signal::INT,
        Signal::Term => rustix::process::Signal::TERM,
        Signal::Kill => rustix::process::Signal::KILL,
    }
}

fn ours(pid: u32) -> Option<Pid> {
    let p = Pid::from_raw(i32::try_from(pid).ok()?)?;
    (pid > 1 && p != getpid() && p != getpgrp()).then_some(p)
}

pub fn may_signal(pid: u32) -> bool {
    ours(pid).is_some()
}

pub fn signal(pid: u32, signal: Signal) -> std::io::Result<()> {
    let p = ours(pid).ok_or_else(|| std::io::Error::from(std::io::ErrorKind::PermissionDenied))?;
    rustix::process::kill_process(p, to_rustix(signal)).map_err(Into::into)
}

pub fn signal_group(pid: u32, signal: Signal) {
    if let Some(p) = ours(pid) {
        let _ = rustix::process::kill_process_group(p, to_rustix(signal));
    }
}

pub fn group_exists(pid: u32) -> bool {
    ours(pid).is_some_and(|p| rustix::process::test_kill_process_group(p).is_ok())
}

pub fn own_session(cmd: &mut Command) {
    // SAFETY: `setsid` is async-signal-safe and touches no memory of the parent; the
    // error is made from its number, without allocating.
    unsafe {
        cmd.pre_exec(|| rustix::process::setsid().map(|_| ()).map_err(Into::into));
    }
}

pub fn terminal_session(cmd: &mut Command) {
    // SAFETY: between fork and exec, only system calls (no allocation, no locks). The
    // terminal is already on 0, 1 and 2: the new session takes it as its own.
    unsafe {
        cmd.pre_exec(|| {
            rustix::process::setsid()?;
            rustix::process::ioctl_tiocsctty(rustix::stdio::stdin())?;
            Ok(())
        });
    }
}
