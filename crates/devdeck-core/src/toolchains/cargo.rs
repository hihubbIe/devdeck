//! Cargo projects: `cargo:build` and `cargo:test`, plus what runs. The manifests are read
//! into a [`Manifest`] (the root's package, the members, which packages `cargo run`
//! picks among by default), then [`runs`] turns it into run commands:
//! - bare `cargo run` (`cargo:run`) when cargo can choose by itself: one binary among
//!   the default packages, or the root's `default-run`;
//! - every other binary gets `cargo:run:<bin>` (`cargo:run:<package>:<bin>` when two
//!   packages have it): `cargo run --bin <bin>`, with `-p <package>` when its package
//!   isn't a default one or the name is shared, and `--features` for those it requires
//!   (with `-p` unless its package is the only default one).
//!
//! The default packages are `workspace.default-members`, else the root's package when
//! there is one, else every member. `workspace.exclude` is honoured. Without a binary, a
//! library.

use super::{Cx, Facts, Listing, Part, Toolchain, glob, quote};
use std::path::Path;

pub struct Cargo;

impl Toolchain for Cargo {
    fn sources(&self) -> &'static [&'static str] {
        &["cargo"]
    }

    fn reads(&self, name: &str) -> bool {
        name == "Cargo.toml"
    }

    fn scan(&self, dir: &Path, listing: &Listing, _parent: Option<&Facts>, cx: &mut Cx) -> Option<Part> {
        if !listing.file("Cargo.toml") {
            return None;
        }
        let mut part = Part::new("cargo");
        let Some(manifest) = Manifest::read(dir, cx) else { return Some(part) };
        let mut commands: Vec<(String, String)> = vec![("build".into(), "cargo build".into()), ("test".into(), "cargo test".into())];
        commands.extend(runs(&manifest));
        part.runnable = Some(manifest.packages().any(|p| !p.bins.is_empty()));
        let names: Vec<&str> = commands.iter().map(|(n, _)| n.as_str()).collect();
        for (name, line) in &commands {
            part.add("cargo", name, line.clone(), &names, &["cargo"]);
        }
        Some(part)
    }
}

/// A binary target.
#[derive(Clone, Debug, PartialEq)]
pub struct Bin {
    pub name: String,
    /// Its `required-features`.
    pub features: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub name: String,
    pub bins: Vec<Bin>,
}

/// What a project's Cargo manifests say about running it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manifest {
    /// The root `Cargo.toml`'s own package.
    pub root: Option<Package>,
    /// The workspace members other than the root's package.
    pub members: Vec<Package>,
    /// The packages `cargo run` picks among without `-p`, by name.
    pub defaults: Vec<String>,
    /// The root package's `default-run`.
    pub default_run: Option<String>,
}

impl Manifest {
    /// From `dir/Cargo.toml` and its members'; `None` when the root one is broken
    /// (reported).
    fn read(dir: &Path, cx: &mut Cx) -> Option<Manifest> {
        let table = manifest(dir, cx)?;
        let root = table.contains_key("package").then(|| package(dir, &table));
        let workspace = table.get("workspace");
        let list = |key: &str| -> Vec<String> {
            let values = workspace.and_then(|w| w.get(key)).and_then(toml::Value::as_array);
            values.into_iter().flatten().filter_map(toml::Value::as_str).map(String::from).collect()
        };
        let excluded = glob::expand(dir, &list("exclude"), cx).dirs;
        let mut members: Vec<(String, Package)> = Vec::new();
        for rel in glob::expand(dir, &list("members"), cx).dirs {
            let member = dir.join(&rel);
            if excluded.contains(&rel) || !member.join("Cargo.toml").is_file() {
                continue;
            }
            if let Some(t) = manifest(&member, cx).filter(|t| t.contains_key("package")) {
                let p = package(&member, &t);
                // A workspace listing its own root (`"."`).
                if root.as_ref().is_some_and(|r| r.name == p.name) || members.iter().any(|(_, m)| m.name == p.name) {
                    continue;
                }
                members.push((rel, p));
            }
        }
        let default_members = list("default-members");
        let defaults: Vec<String> = if !default_members.is_empty() {
            let chosen = glob::expand(dir, &default_members, cx).dirs;
            let mut d: Vec<String> = members.iter().filter(|(rel, _)| chosen.contains(rel)).map(|(_, p)| p.name.clone()).collect();
            if let Some(r) = root.as_ref().filter(|_| default_members.iter().any(|m| m == ".")) {
                d.insert(0, r.name.clone());
            }
            d
        } else if let Some(r) = &root {
            vec![r.name.clone()]
        } else {
            members.iter().map(|(_, p)| p.name.clone()).collect()
        };
        let default_run = table.get("package").and_then(|p| p.get("default-run")).and_then(toml::Value::as_str).map(String::from);
        Some(Manifest { root, members: members.into_iter().map(|(_, p)| p).collect(), defaults, default_run })
    }

    fn packages(&self) -> impl Iterator<Item = &Package> {
        self.root.iter().chain(&self.members)
    }
}

/// The run commands of a project: `(name, line)`, `run` for bare `cargo run`.
pub fn runs(m: &Manifest) -> Vec<(String, String)> {
    let bins: Vec<(&Package, &Bin)> = m.packages().flat_map(|p| p.bins.iter().map(move |b| (p, b))).collect();
    let default = |p: &Package| m.defaults.contains(&p.name);
    let in_defaults: Vec<(&Package, &Bin)> = bins.iter().copied().filter(|(p, _)| default(p)).collect();
    // What bare `cargo run` runs, when cargo can choose.
    let chosen = match (&m.default_run, in_defaults.as_slice()) {
        (Some(run), _) if m.defaults.len() == 1 => in_defaults.iter().copied().find(|(_, b)| b.name == *run),
        (_, [only]) => Some(*only),
        _ => None,
    }
    .filter(|(_, b)| b.features.is_empty());
    let mut out = Vec::new();
    if chosen.is_some() {
        out.push(("run".to_string(), "cargo run".to_string()));
    }
    for &(p, b) in &bins {
        if chosen.is_some_and(|(cp, cb)| cp.name == p.name && cb.name == b.name) {
            continue;
        }
        let shared = bins.iter().filter(|(_, o)| o.name == b.name).count() > 1;
        let only_default = m.defaults.len() == 1 && default(p);
        let mut line = String::from("cargo run");
        if !default(p) || shared || (!b.features.is_empty() && !only_default) {
            line += &format!(" -p {}", quote(&p.name));
        }
        line += &format!(" --bin {}", quote(&b.name));
        if !b.features.is_empty() {
            line += &format!(" --features {}", quote(&b.features.join(",")));
        }
        let name = if shared { format!("run:{}:{}", p.name, b.name) } else { format!("run:{}", b.name) };
        out.push((name, line));
    }
    out
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

/// A package and its binaries: its `[[bin]]` targets, and unless `autobins = false`,
/// `src/main.rs` (named after the package) and `src/bin/*.rs` or `src/bin/*/main.rs`,
/// except those a `[[bin]]` already names or points at (as cargo does).
fn package(dir: &Path, table: &toml::Table) -> Package {
    let name = table.get("package").and_then(|p| p.get("name")).and_then(toml::Value::as_str).unwrap_or_default().to_string();
    let mut bins: Vec<Bin> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for b in table.get("bin").and_then(toml::Value::as_array).into_iter().flatten() {
        let bin = b.get("name").and_then(toml::Value::as_str).unwrap_or(&name).to_string();
        if let Some(p) = b.get("path").and_then(toml::Value::as_str) {
            paths.push(p.trim_start_matches("./").to_string());
        }
        let features = b.get("required-features").and_then(toml::Value::as_array).into_iter().flatten().filter_map(toml::Value::as_str).map(String::from).collect();
        bins.push(Bin { name: bin, features });
    }
    let auto = table.get("package").and_then(|p| p.get("autobins")).and_then(toml::Value::as_bool).unwrap_or(true);
    if auto {
        // Name and path of each inferred binary.
        let mut found: Vec<(String, String)> = Vec::new();
        if dir.join("src/main.rs").is_file() {
            found.push((name.clone(), "src/main.rs".into()));
        }
        if let Ok(entries) = std::fs::read_dir(dir.join("src/bin")) {
            let mut names: Vec<(String, String)> = entries
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let file = e.file_name().into_string().ok()?;
                    match file.strip_suffix(".rs") {
                        Some(stem) if path.is_file() => Some((stem.to_string(), format!("src/bin/{file}"))),
                        _ if path.join("main.rs").is_file() => Some((file.clone(), format!("src/bin/{file}/main.rs"))),
                        _ => None,
                    }
                })
                .collect();
            names.sort();
            found.extend(names);
        }
        for (bin, path) in found {
            if !bins.iter().any(|b| b.name == bin) && !paths.contains(&path) {
                bins.push(Bin { name: bin, features: Vec::new() });
            }
        }
    }
    Package { name, bins }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::Tier;
    use crate::testing::{app, lines, tier};
    use crate::testing::Tree;

    fn bin(name: &str, features: &[&str]) -> Bin {
        Bin { name: name.into(), features: features.iter().map(|f| f.to_string()).collect() }
    }

    fn pkg(name: &str, bins: Vec<Bin>) -> Package {
        Package { name: name.into(), bins }
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn bare_cargo_run_when_cargo_can_choose() {
        let one = Manifest { root: Some(pkg("app", vec![bin("app", &[])])), defaults: vec!["app".into()], ..Manifest::default() };
        assert_eq!(runs(&one), pairs(&[("run", "cargo run")]));
        let none = Manifest { root: Some(pkg("lib", vec![])), defaults: vec!["lib".into()], ..Manifest::default() };
        assert_eq!(runs(&none), [], "a library");
        let chosen = Manifest {
            root: Some(pkg("c", vec![bin("a", &[]), bin("b", &[])])),
            defaults: vec!["c".into()],
            default_run: Some("a".into()),
            ..Manifest::default()
        };
        assert_eq!(runs(&chosen), pairs(&[("run", "cargo run"), ("run:b", "cargo run --bin b")]), "default-run");
    }

    #[test]
    fn default_members_decide_and_the_others_need_their_package() {
        // `game`: a virtual workspace with `default-members = ["game"]`.
        let game = Manifest {
            members: vec![pkg("engine", vec![]), pkg("game", vec![bin("game", &[])]), pkg("xtask", vec![bin("xtask", &[])])],
            defaults: vec!["game".into()],
            ..Manifest::default()
        };
        assert_eq!(runs(&game), pairs(&[("run", "cargo run"), ("run:xtask", "cargo run -p xtask --bin xtask")]));
        // A root package with members: cargo runs the root's.
        let rooted = Manifest {
            root: Some(pkg("main", vec![bin("main", &[])])),
            members: vec![pkg("tool", vec![bin("tool", &[])])],
            defaults: vec!["main".into()],
            ..Manifest::default()
        };
        assert_eq!(runs(&rooted), pairs(&[("run", "cargo run"), ("run:tool", "cargo run -p tool --bin tool")]));
    }

    #[test]
    fn several_binaries_each_get_a_command() {
        // devdeck: a virtual workspace, every member a default one.
        let devdeck = Manifest {
            members: vec![pkg("core", vec![]), pkg("app", vec![bin("devdeck", &[]), bin("devdeck-snapshot", &["snapshot"])])],
            defaults: vec!["core".into(), "app".into()],
            ..Manifest::default()
        };
        assert_eq!(
            runs(&devdeck),
            pairs(&[("run:devdeck", "cargo run --bin devdeck"), ("run:devdeck-snapshot", "cargo run -p app --bin devdeck-snapshot --features snapshot")]),
            "features with -p: several default packages"
        );
        let shared = Manifest {
            members: vec![pkg("tool", vec![bin("tool", &[])]), pkg("extra", vec![bin("tool", &[]), bin("gen", &[])])],
            defaults: vec!["tool".into(), "extra".into()],
            ..Manifest::default()
        };
        assert_eq!(
            runs(&shared),
            pairs(&[("run:tool:tool", "cargo run -p tool --bin tool"), ("run:extra:tool", "cargo run -p extra --bin tool"), ("run:gen", "cargo run --bin gen")])
        );
        let lone = Manifest { root: Some(pkg("one", vec![bin("one", &["cli"])])), defaults: vec!["one".into()], ..Manifest::default() };
        assert_eq!(runs(&lone), pairs(&[("run:one", "cargo run --bin one --features cli")]), "the only default package: no -p");
    }

    #[test]
    fn manifests_are_read_as_cargo_reads_them() {
        let t = Tree::new("cargo-read");
        t.file("ws/Cargo.toml", "[workspace]\nmembers = [\".\", \"crates/*\"]\nexclude = [\"crates/old\"]\ndefault-members = [\"crates/app\"]\n[package]\nname = \"ws\"\n")
            .file("ws/src/main.rs", "")
            .file("ws/crates/app/Cargo.toml", "[package]\nname = \"app\"\n[[bin]]\nname = \"app\"\npath = \"src/main.rs\"\n")
            .file("ws/crates/app/src/main.rs", "")
            .file("ws/crates/app/src/bin/gen/main.rs", "")
            .file("ws/crates/old/Cargo.toml", "[package]\nname = \"old\"\n")
            .file("ws/crates/old/src/main.rs", "");
        let found = t.scan();
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let ws = found.apps.iter().find(|a| a.id == "ws").unwrap();
        let runs: Vec<(&str, &str)> = ws.commands.iter().filter(|c| c.id.starts_with("cargo:run")).map(|c| (c.id.as_str(), c.line.as_str())).collect();
        assert_eq!(
            runs,
            [("cargo:run:app", "cargo run --bin app"), ("cargo:run:gen", "cargo run --bin gen"), ("cargo:run:ws", "cargo run -p ws --bin ws")],
            "default-members: app; the root listed as \".\" counted once; old excluded; [[bin]] path drops the inferred main"
        );
    }

    #[test]
    fn cargo_projects_run_when_they_have_a_binary() {
        let t = Tree::new("cargo");
        t.file("bin/Cargo.toml", "[package]\nname = \"bin\"\n").file("bin/src/main.rs", "fn main() {}\n");
        t.file("lib/Cargo.toml", "[package]\nname = \"lib\"\n").file("lib/src/lib.rs", "");
        t.file("virt/Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n")
            .file("virt/crates/core/Cargo.toml", "[package]\nname = \"core\"\n")
            .file("virt/crates/app/Cargo.toml", "[package]\nname = \"app\"\n[[bin]]\nname = \"app\"\npath = \"main.rs\"\n");
        let found = t.scan();
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let bin = app(&found, "bin");
        assert_eq!((bin.kind.as_str(), bin.library), ("cargo", false));
        assert_eq!(lines(bin), [("cargo:build", "cargo build"), ("cargo:run", "cargo run"), ("cargo:test", "cargo test")]);
        assert_eq!(tier(bin, "cargo:run"), Tier::Primary);
        assert_eq!(bin.commands[0].tools, ["cargo"]);
        let lib = app(&found, "lib");
        assert!(lib.library);
        assert_eq!(lines(lib), [("cargo:build", "cargo build"), ("cargo:test", "cargo test")]);
        assert!(!app(&found, "virt").library, "a member has a binary");
        assert!(app(&found, "virt").command("cargo:run").is_some());
    }

    #[test]
    fn cargo_runs_one_command_per_binary_when_cargo_run_cant_choose() {
        let t = Tree::new("cargo-bins");
        t.file("ws/Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n")
            .file("ws/crates/core/Cargo.toml", "[package]\nname = \"core\"\n")
            .file(
                "ws/crates/app/Cargo.toml",
                "[package]\nname = \"app\"\n[[bin]]\nname = \"devdeck\"\npath = \"src/main.rs\"\n[[bin]]\nname = \"devdeck-snapshot\"\npath = \"src/bin/snapshot.rs\"\nrequired-features = [\"snapshot\"]\n",
            )
            .file("ws/crates/app/src/main.rs", "fn main() {}\n")
            .file("ws/crates/tool/Cargo.toml", "[package]\nname = \"tool\"\n")
            .file("ws/crates/tool/src/main.rs", "fn main() {}\n")
            .file("ws/crates/extra/Cargo.toml", "[package]\nname = \"extra\"\n")
            .file("ws/crates/extra/src/bin/tool.rs", "fn main() {}\n")
            .file("ws/crates/extra/src/bin/gen/main.rs", "fn main() {}\n");
        t.file("one/Cargo.toml", "[package]\nname = \"one\"\n[[bin]]\nname = \"one\"\npath = \"main.rs\"\nrequired-features = [\"cli\"]\n");
        t.file("chosen/Cargo.toml", "[package]\nname = \"chosen\"\ndefault-run = \"a\"\n").file("chosen/src/bin/a.rs", "").file("chosen/src/bin/b.rs", "");
        let found = t.scan();
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let runs = |id: &str| lines(app(&found, id)).into_iter().filter(|(c, _)| c.starts_with("cargo:run")).collect::<Vec<_>>();
        assert_eq!(
            runs("ws"),
            [
                ("cargo:run:devdeck", "cargo run --bin devdeck"),
                ("cargo:run:devdeck-snapshot", "cargo run -p app --bin devdeck-snapshot --features snapshot"),
                ("cargo:run:extra:tool", "cargo run -p extra --bin tool"),
                ("cargo:run:gen", "cargo run --bin gen"),
                ("cargo:run:tool:tool", "cargo run -p tool --bin tool"),
            ]
        );
        assert_eq!(tier(app(&found, "ws"), "cargo:run:devdeck"), Tier::Primary);
        assert_eq!(runs("one"), [("cargo:run:one", "cargo run --bin one --features cli")], "a lone binary needing a feature");
        assert_eq!(runs("chosen"), [("cargo:run", "cargo run"), ("cargo:run:b", "cargo run --bin b")], "default-run chooses; the others get theirs");
    }
}
