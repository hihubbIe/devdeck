//! Nothing runs in a terminal here (macOS until step 2c): the types can't be made, so
//! nothing past [`Terminal::open`] or [`Watch::new`] is ever reached.

use super::{ExitStatus, Ready};
use crate::{SysError, Support};
use std::os::fd::OwnedFd;
use std::process::Child;
use std::time::Duration;

const WHY: &str = "running in a terminal isn't supported on this OS yet";

pub const SUPPORT: Support = Support::No(WHY);

pub enum Terminal {}

impl Terminal {
    pub fn open(_rows: u16, _cols: u16) -> Result<(Terminal, OwnedFd), SysError> {
        Err(SysError::Unsupported(WHY))
    }

    pub fn resize(&self, _rows: u16, _cols: u16) {
        match *self {}
    }

    pub fn foreground_group(&self) -> Option<u32> {
        match *self {}
    }

    pub fn set_nonblocking(&self) {
        match *self {}
    }

    pub fn read(&self, _buf: &mut [u8]) -> std::io::Result<usize> {
        match *self {}
    }

    pub fn write(&self, _bytes: &[u8]) -> std::io::Result<usize> {
        match *self {}
    }
}

pub enum Watch {}

pub enum Waker {}

impl Watch {
    pub fn new(_child: &Child) -> Result<Watch, SysError> {
        Err(SysError::Unsupported(WHY))
    }

    pub fn waker(&self) -> Waker {
        match *self {}
    }

    pub fn wait(&self, _terminal: Option<&Terminal>, _out: bool, _child: bool, _timeout: Option<Duration>) -> std::io::Result<Ready> {
        match *self {}
    }

    pub fn drain_wake(&self) {
        match *self {}
    }

    pub fn reap(&self, _wait: bool) -> ExitStatus {
        match *self {}
    }
}

impl Waker {
    pub fn wake(&self) {
        match *self {}
    }
}
