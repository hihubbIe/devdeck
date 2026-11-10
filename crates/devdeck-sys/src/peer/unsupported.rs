use super::Refused;
use crate::Support;
use std::os::unix::net::UnixStream;

const WHY: &str = "callers can't be checked on this OS";

pub const SUPPORT: Support = Support::No(WHY);

pub enum Pin {}

pub fn of(_stream: &UnixStream) -> Result<Option<(u32, Pin)>, Refused> {
    Err(Refused::Unsupported(WHY))
}

pub fn ended(pin: &Pin) -> bool {
    match *pin {}
}
