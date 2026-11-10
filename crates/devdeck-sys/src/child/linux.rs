//! The terminal through `openpt`; the child through a pidfd (`pidfd_open`, `waitid` on
//! it); the wake through an eventfd, whose counter makes several wakes one; one `poll`
//! over the three.

use super::{ExitStatus, Ready};
use crate::{SysError, Support};
use rustix::event::{EventfdFlags, PollFd, PollFlags, Timespec};
use rustix::fd::{AsFd, OwnedFd};
use rustix::process::{Pid, WaitId, WaitIdOptions};
use std::process::Child;
use std::sync::Arc;
use std::time::Duration;

pub const SUPPORT: Support = Support::Yes;

pub struct Terminal(OwnedFd);

impl Terminal {
    pub fn open(rows: u16, cols: u16) -> Result<(Terminal, OwnedFd), SysError> {
        use rustix::fs::{Mode, OFlags};
        use rustix::pty::OpenptFlags;
        let master = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).map_err(io)?;
        rustix::pty::grantpt(&master).map_err(io)?;
        rustix::pty::unlockpt(&master).map_err(io)?;
        let name = rustix::pty::ptsname(&master, Vec::new()).map_err(io)?;
        let other = rustix::fs::open(name.as_c_str(), OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC, Mode::empty()).map_err(io)?;
        rustix::termios::tcsetwinsize(&master, winsize(rows, cols)).map_err(io)?;
        if let Ok(mut t) = rustix::termios::tcgetattr(&master) {
            t.input_modes.insert(rustix::termios::InputModes::IUTF8);
            let _ = rustix::termios::tcsetattr(&master, rustix::termios::OptionalActions::Now, &t);
        }
        Ok((Terminal(master), other))
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let _ = rustix::termios::tcsetwinsize(&self.0, winsize(rows, cols));
    }

    pub fn foreground_group(&self) -> Option<u32> {
        Some(rustix::termios::tcgetpgrp(&self.0).ok()?.as_raw_nonzero().get() as u32)
    }

    pub fn set_nonblocking(&self) {
        let _ = rustix::io::ioctl_fionbio(&self.0, true);
    }

    pub fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        rustix::io::read(&self.0, buf).map_err(Into::into)
    }

    pub fn write(&self, bytes: &[u8]) -> std::io::Result<usize> {
        rustix::io::write(&self.0, bytes).map_err(Into::into)
    }
}

fn winsize(rows: u16, cols: u16) -> rustix::termios::Winsize {
    rustix::termios::Winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 }
}

fn io(e: rustix::io::Errno) -> SysError {
    SysError::Io(e.into())
}

pub struct Watch {
    pidfd: OwnedFd,
    wake: Arc<OwnedFd>,
}

pub struct Waker(Arc<OwnedFd>);

impl Watch {
    pub fn new(child: &Child) -> Result<Watch, SysError> {
        let pidfd = rustix::process::pidfd_open(Pid::from_child(child), rustix::process::PidfdFlags::empty()).map_err(io)?;
        let wake = rustix::event::eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).map_err(io)?;
        Ok(Watch { pidfd, wake: Arc::new(wake) })
    }

    pub fn waker(&self) -> Waker {
        Waker(self.wake.clone())
    }

    pub fn wait(&self, terminal: Option<&Terminal>, out: bool, child: bool, timeout: Option<Duration>) -> std::io::Result<Ready> {
        let mut fds = vec![PollFd::new(&*self.wake, PollFlags::IN)];
        if child {
            fds.push(PollFd::new(&self.pidfd, PollFlags::IN));
        }
        if let Some(t) = terminal {
            let out = if out { PollFlags::OUT } else { PollFlags::empty() };
            fds.push(PollFd::new(&t.0, PollFlags::IN | out));
        }
        let timeout = timeout.map(|d| Timespec { tv_sec: d.as_secs() as _, tv_nsec: d.subsec_nanos() as _ });
        rustix::event::poll(&mut fds, timeout.as_ref())?;
        let term = if terminal.is_some() { fds[fds.len() - 1].revents() } else { PollFlags::empty() };
        Ok(Ready {
            wake: !fds[0].revents().is_empty(),
            child: child && !fds[1].revents().is_empty(),
            readable: term.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR),
            writable: term.contains(PollFlags::OUT),
        })
    }

    pub fn drain_wake(&self) {
        let _ = rustix::io::read(&*self.wake, &mut [0u8; 8]);
    }

    pub fn reap(&self, wait: bool) -> ExitStatus {
        let options = if wait { WaitIdOptions::EXITED } else { WaitIdOptions::EXITED | WaitIdOptions::NOHANG };
        match rustix::process::waitid(WaitId::PidFd(self.pidfd.as_fd()), options) {
            Ok(Some(status)) => match status.terminating_signal() {
                Some(signal) => ExitStatus::Signal(signal),
                None => ExitStatus::Code(status.exit_status().unwrap_or(-1)),
            },
            _ => ExitStatus::Code(-1),
        }
    }
}

impl Waker {
    pub fn wake(&self) {
        let _ = rustix::io::write(&*self.0, &1u64.to_ne_bytes());
    }
}
