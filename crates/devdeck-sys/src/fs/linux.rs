use crate::{SysError, Support};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

pub const OPEN_PATH_SUPPORT: Support = Support::Yes;

/// The link `/proc/self/fd/<fd>` names the file's path.
pub fn open_path(file: &File) -> Result<PathBuf, SysError> {
    Ok(std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?)
}
