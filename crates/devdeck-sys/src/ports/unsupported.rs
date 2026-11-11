use super::{Holder, Listener, Listening};
use crate::{SysError, Support};

const WHY: &str = "ports aren't found on this OS yet";

pub const SUPPORT: Support = Support::No(WHY);

pub fn listening(_pids: &[u32]) -> Result<Vec<Listening>, SysError> {
    Err(SysError::Unsupported(WHY))
}

pub fn all() -> Result<Vec<Listener>, SysError> {
    Err(SysError::Unsupported(WHY))
}

pub fn holder(_port: u16) -> Result<Option<Holder>, SysError> {
    Err(SysError::Unsupported(WHY))
}
