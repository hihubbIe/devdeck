use super::Memory;
use crate::{SysError, Support};

const WHY: &str = "devdeck's memory isn't measured on this OS yet";

pub const SUPPORT: Support = Support::No(WHY);

pub fn footprint() -> Result<Memory, SysError> {
    Err(SysError::Unsupported(WHY))
}
