use super::Proc;
use crate::{SysError, Support};

const WHY: &str = "the process table isn't read on this OS yet";

pub const SUPPORT: Support = Support::No(WHY);

pub fn get(_pid: u32) -> Option<Proc> {
    None
}

pub fn parents() -> Result<Vec<(u32, u32)>, SysError> {
    Err(SysError::Unsupported(WHY))
}

pub fn name(_pid: u32) -> Option<String> {
    None
}

pub fn command(_pid: u32) -> Option<String> {
    None
}
