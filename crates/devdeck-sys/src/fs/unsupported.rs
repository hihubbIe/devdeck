use crate::{SysError, Support};
use std::fs::File;
use std::path::PathBuf;

const WHY: &str = "an open file's path isn't known on this OS yet";

pub const OPEN_PATH_SUPPORT: Support = Support::No(WHY);

pub fn open_path(_file: &File) -> Result<PathBuf, SysError> {
    Err(SysError::Unsupported(WHY))
}
