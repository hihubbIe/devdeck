use crate::{SysError, Support};
use std::path::PathBuf;

const WHY: &str = "no folder for the control API's socket on this OS yet";

pub const SUPPORT: Support = Support::No(WHY);

pub fn runtime(_env: &impl Fn(&str) -> Option<String>) -> Result<PathBuf, SysError> {
    Err(SysError::Unsupported(WHY))
}
