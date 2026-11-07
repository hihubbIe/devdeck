//! Docker compose files: `compose:up` and `compose:down` for the default file, and
//! `compose:up:<x>` / `compose:down:<x>` with `-f` for `docker-compose.<x>.yml` or
//! `compose.<x>.yaml` (an `override` file is merged into the default one by docker).

use super::{Cx, Listing, Part, quote, yaml};
use serde::de::IgnoredAny;
use std::path::Path;

/// The names `docker compose` looks for, in its order.
pub(super) const DEFAULTS: [&str; 4] = ["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"];

pub(super) fn scan(dir: &Path, listing: &Listing, cx: &mut Cx) -> Option<Part> {
    // (suffix, file, -f needed)
    let mut files: Vec<(String, &str, bool)> = Vec::new();
    if let Some(name) = DEFAULTS.into_iter().find(|n| listing.file(n)) {
        files.push((String::new(), name, false));
    }
    for name in listing.files() {
        if let Some(x) = variant(name) {
            files.push((format!(":{x}"), name, true));
        }
    }
    if files.is_empty() {
        return None;
    }
    let mut part = Part::new("docker");
    let valid: Vec<&(String, &str, bool)> = files
        .iter()
        .filter(|(_, name, _)| {
            let path = dir.join(name);
            let Some(text) = cx.read(&path) else { return false };
            yaml::<IgnoredAny>(&text).map_err(|e| cx.problem(&path, e)).is_ok()
        })
        .collect();
    let names: Vec<String> = valid.iter().flat_map(|(x, _, _)| [format!("up{x}"), format!("down{x}")]).collect();
    let siblings: Vec<&str> = names.iter().map(String::as_str).collect();
    for (x, file, explicit) in valid {
        let base = if *explicit { format!("docker compose -f {}", quote(file)) } else { "docker compose".to_string() };
        for verb in ["up", "down"] {
            part.add("compose", &format!("{verb}{x}"), format!("{base} {verb}"), &siblings, &["docker"]);
        }
    }
    Some(part)
}

/// `dev` for `docker-compose.dev.yml` or `compose.dev.yaml`; not for the override file.
pub(super) fn variant(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".yml").or_else(|| name.strip_suffix(".yaml"))?;
    let x = stem.strip_prefix("docker-compose.").or_else(|| stem.strip_prefix("compose."))?;
    (!x.is_empty() && x != "override").then_some(x)
}
