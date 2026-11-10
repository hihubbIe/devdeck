use crate::{SysError, Support};
use std::path::PathBuf;

pub const SUPPORT: Support = Support::Yes;

/// `$XDG_RUNTIME_DIR/devdeck`: the session's own folder, gone at logout.
pub fn runtime(env: &impl Fn(&str) -> Option<String>) -> Result<PathBuf, SysError> {
    env("XDG_RUNTIME_DIR").filter(|s| !s.is_empty()).map(|d| PathBuf::from(d).join("devdeck")).ok_or(SysError::Unsupported("XDG_RUNTIME_DIR isn't set"))
}
