use std::path::PathBuf;

fn base(env: &impl Fn(&str) -> Option<String>, var: &str, fallback: &[&str]) -> PathBuf {
    if let Some(d) = env(var).filter(|s| !s.is_empty()) {
        return PathBuf::from(d).join("devdeck");
    }
    let mut p = PathBuf::from(env("HOME").unwrap_or_else(|| ".".into()));
    p.extend(fallback);
    p.join("devdeck")
}

/// `$XDG_CONFIG_HOME/devdeck`, else `~/.config/devdeck`.
pub fn config(env: &impl Fn(&str) -> Option<String>) -> PathBuf {
    base(env, "XDG_CONFIG_HOME", &[".config"])
}

/// `$XDG_STATE_HOME/devdeck`, else `~/.local/state/devdeck`.
pub fn state(env: &impl Fn(&str) -> Option<String>) -> PathBuf {
    base(env, "XDG_STATE_HOME", &[".local", "state"])
}

/// `$XDG_DATA_HOME/devdeck`, else `~/.local/share/devdeck`.
pub fn data(env: &impl Fn(&str) -> Option<String>) -> PathBuf {
    base(env, "XDG_DATA_HOME", &[".local", "share"])
}
