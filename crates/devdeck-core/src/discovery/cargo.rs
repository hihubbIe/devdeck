//! Cargo projects: `cargo:build` and `cargo:test`, plus what runs. `cargo run` alone
//! works only when it can tell which binary to run: one binary in the package or the
//! workspace, or a `default-run`. Otherwise each binary gets `cargo:run:<bin>`
//! (`cargo run --bin <bin>`, with `-p` when two packages have it, and the features it
//! requires). Without a binary, a library.

use super::{Cx, Listing, Part, glob, quote};
use std::path::Path;

/// A binary target.
#[derive(Debug, PartialEq)]
struct Bin {
    name: String,
    package: String,
    /// Its `required-features`.
    features: Vec<String>,
}

pub(super) fn scan(dir: &Path, listing: &Listing, cx: &mut Cx) -> Option<Part> {
    if !listing.file("Cargo.toml") {
        return None;
    }
    let mut part = Part::new("cargo");
    let Some(table) = manifest(dir, cx) else { return Some(part) };
    let mut bins = if table.contains_key("package") { bins(dir, &table) } else { Vec::new() };
    let members = table.get("workspace").and_then(|w| w.get("members")).and_then(toml::Value::as_array);
    if let Some(members) = members {
        let patterns: Vec<String> = members.iter().filter_map(toml::Value::as_str).map(String::from).collect();
        for rel in glob::expand(dir, &patterns, cx).dirs {
            let member = dir.join(rel);
            if member.join("Cargo.toml").is_file()
                && let Some(t) = manifest(&member, cx)
            {
                bins.extend(self::bins(&member, &t));
            }
        }
    }
    // A workspace listing its own root as a member.
    bins.dedup();
    let default_run = table.get("package").and_then(|p| p.get("default-run")).is_some();
    let mut runs: Vec<(String, String)> = Vec::new();
    match bins.as_slice() {
        [] => {}
        [only] if only.features.is_empty() => runs.push(("run".into(), "cargo run".into())),
        _ if default_run => runs.push(("run".into(), "cargo run".into())),
        _ => {
            for b in &bins {
                let twice = bins.iter().filter(|o| o.name == b.name).count() > 1;
                let mut line = String::from("cargo run");
                if twice {
                    line += &format!(" -p {}", quote(&b.package));
                }
                line += &format!(" --bin {}", quote(&b.name));
                if !b.features.is_empty() {
                    line += &format!(" --features {}", quote(&b.features.join(",")));
                }
                let id = if twice { format!("run:{}:{}", b.package, b.name) } else { format!("run:{}", b.name) };
                runs.push((id, line));
            }
        }
    }
    let mut commands: Vec<(String, String)> = vec![("build".into(), "cargo build".into()), ("test".into(), "cargo test".into())];
    let runnable = !runs.is_empty();
    commands.extend(runs);
    let names: Vec<&str> = commands.iter().map(|(n, _)| n.as_str()).collect();
    for (name, line) in &commands {
        part.add("cargo", name, line.clone(), &names, &["cargo"]);
    }
    part.runnable = Some(runnable);
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

/// A package's binaries: its `[[bin]]` targets, and unless `autobins = false`,
/// `src/main.rs` (named after the package) and `src/bin/*.rs` or `src/bin/*/main.rs`,
/// except those a `[[bin]]` already names or points at (as cargo does).
fn bins(dir: &Path, table: &toml::Table) -> Vec<Bin> {
    let package = table.get("package").and_then(|p| p.get("name")).and_then(toml::Value::as_str).unwrap_or_default().to_string();
    let mut out: Vec<Bin> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for b in table.get("bin").and_then(toml::Value::as_array).into_iter().flatten() {
        let name = b.get("name").and_then(toml::Value::as_str).unwrap_or(&package).to_string();
        if let Some(p) = b.get("path").and_then(toml::Value::as_str) {
            paths.push(p.trim_start_matches("./").to_string());
        }
        let features = b.get("required-features").and_then(toml::Value::as_array).into_iter().flatten().filter_map(toml::Value::as_str).map(String::from).collect();
        out.push(Bin { name, package: package.clone(), features });
    }
    let auto = table.get("package").and_then(|p| p.get("autobins")).and_then(toml::Value::as_bool).unwrap_or(true);
    if auto {
        // Name and path of each inferred binary.
        let mut found: Vec<(String, String)> = Vec::new();
        if dir.join("src/main.rs").is_file() {
            found.push((package.clone(), "src/main.rs".into()));
        }
        if let Ok(entries) = std::fs::read_dir(dir.join("src/bin")) {
            let mut names: Vec<(String, String)> = entries
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let name = e.file_name().into_string().ok()?;
                    match name.strip_suffix(".rs") {
                        Some(stem) if path.is_file() => Some((stem.to_string(), format!("src/bin/{name}"))),
                        _ if path.join("main.rs").is_file() => Some((name.clone(), format!("src/bin/{name}/main.rs"))),
                        _ => None,
                    }
                })
                .collect();
            names.sort();
            found.extend(names);
        }
        for (name, path) in found {
            if !out.iter().any(|b| b.name == name) && !paths.contains(&path) {
                out.push(Bin { name, package: package.clone(), features: Vec::new() });
            }
        }
    }
    out
}
