//! Docker compose files: `compose:up` and `compose:down` for the default file, and
//! `compose:up:<x>` / `compose:down:<x>` with `-f` for `docker-compose.<x>.yml` or
//! `compose.<x>.yaml` (an `override` file is merged into the default one by docker).

use super::{Cx, Facts, Listing, Part, Toolchain, quote, yaml};
use serde::de::IgnoredAny;
use std::path::Path;

/// The names `docker compose` looks for, in its order.
const DEFAULTS: [&str; 4] = ["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"];

pub struct Compose;

impl Toolchain for Compose {
    fn sources(&self) -> &'static [&'static str] {
        &["compose"]
    }

    fn reads(&self, name: &str) -> bool {
        DEFAULTS.contains(&name) || variant(name).is_some()
    }

    fn scan(&self, dir: &Path, listing: &Listing, _parent: Option<&Facts>, cx: &mut Cx) -> Option<Part> {
        scan(dir, listing, cx)
    }
}

fn scan(dir: &Path, listing: &Listing, cx: &mut Cx) -> Option<Part> {
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
fn variant(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".yml").or_else(|| name.strip_suffix(".yaml"))?;
    let x = stem.strip_prefix("docker-compose.").or_else(|| stem.strip_prefix("compose."))?;
    (!x.is_empty() && x != "override").then_some(x)
}

#[cfg(test)]
mod tests {
    use crate::apps::Tier;
    use crate::testing::{Tree, app, lines, tier};

    #[test]
    fn compose_files_give_up_and_down() {
        let t = Tree::new("compose");
        t.file("stack/docker-compose.yml", "services:\n  db:\n    image: postgres\n")
            .file("stack/docker-compose.override.yml", "services: {}\n")
            .file("stack/docker-compose.dev.yml", "services: {}\n")
            .file("stack/compose.broken.yaml", "services: [\n");
        let found = t.scan();
        let stack = app(&found, "stack");
        assert_eq!((stack.kind.as_str(), stack.library), ("docker", false));
        assert_eq!(
            lines(stack),
            [
                ("compose:down", "docker compose down"),
                ("compose:down:dev", "docker compose -f docker-compose.dev.yml down"),
                ("compose:up", "docker compose up"),
                ("compose:up:dev", "docker compose -f docker-compose.dev.yml up"),
            ]
        );
        assert_eq!(tier(stack, "compose:up:dev"), Tier::Primary);
        assert_eq!(stack.command("compose:up").unwrap().tools, ["docker"]);
        assert_eq!(found.problems.len(), 1, "{:?}", found.problems);
        assert!(found.problems[0].contains("stack/compose.broken.yaml: "), "{:?}", found.problems);
    }
}
