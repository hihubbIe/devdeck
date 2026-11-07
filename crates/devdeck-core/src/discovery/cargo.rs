//! Cargo projects: `cargo:build` and `cargo:test`, plus `cargo:run` when there is a
//! binary (in the package or, for a workspace, in a member). Without one, a library.

use super::{Cx, Listing, Part, glob};
use std::path::Path;

pub(super) fn scan(dir: &Path, listing: &Listing, cx: &mut Cx) -> Option<Part> {
    if !listing.file("Cargo.toml") {
        return None;
    }
    let mut part = Part::new("cargo");
    let Some(table) = manifest(dir, cx) else { return Some(part) };
    let mut bin = table.contains_key("package") && has_bin(dir, &table);
    let members = table.get("workspace").and_then(|w| w.get("members")).and_then(toml::Value::as_array);
    if !bin && let Some(members) = members {
        let patterns: Vec<String> = members.iter().filter_map(toml::Value::as_str).map(String::from).collect();
        for rel in glob::expand(dir, &patterns, cx).dirs {
            let member = dir.join(rel);
            if member.join("Cargo.toml").is_file() && manifest(&member, cx).is_some_and(|t| has_bin(&member, &t)) {
                bin = true;
                break;
            }
        }
    }
    let names: &[&str] = if bin { &["build", "run", "test"] } else { &["build", "test"] };
    for name in names {
        part.add("cargo", name, format!("cargo {name}"), names, &["cargo"]);
    }
    part.runnable = Some(bin);
    Some(part)
}

/// `dir/Cargo.toml`; a broken one is a problem.
fn manifest(dir: &Path, cx: &mut Cx) -> Option<toml::Table> {
    let path = dir.join("Cargo.toml");
    let text = cx.read(&path)?;
    match toml::from_str::<toml::Table>(&text) {
        Ok(t) => Some(t),
        Err(e) => {
            let line = e.span().map(|s| text[..s.start.min(text.len())].matches('\n').count() + 1);
            let at = line.map(|l| format!("line {l}: ")).unwrap_or_default();
            cx.problem(&path, format!("{at}{}", e.message().trim()));
            None
        }
    }
}

/// Whether a package builds a binary.
fn has_bin(dir: &Path, table: &toml::Table) -> bool {
    table.get("bin").and_then(toml::Value::as_array).is_some_and(|b| !b.is_empty())
        || dir.join("src/main.rs").is_file()
        || dir.join("src/bin").is_dir()
}
