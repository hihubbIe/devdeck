//! The OS stays here (R11.6, `docs/PLATFORMS.md` §4): no source file of another crate
//! names Linux's files, its runtime folder, systemd, or the crates that call the OS.
//! Line comments are skipped. The manifests guard the rest: no crate but this one
//! depends on the crates that call the OS.

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &["/proc", "/sys/", "XDG_RUNTIME_DIR", "systemd-run", "std::os::linux", "rustix::", "libc::", "signal_hook"];

/// Files still to move, each with the step of `docs/plans/macos.md` that moves it: none
/// since step 1d. A file that no longer names any of the above must leave the list.
const NOT_YET: &[(&str, &str)] = &[];

/// Crates that call the OS: dependencies of this crate only.
const OS_CRATES: &[&str] = &["rustix", "libc", "nix", "signal-hook"];

fn crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates/").to_path_buf()
}

/// Every Rust file of the other crates' `src`, `tests` and `benches`, and their `build.rs`.
fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for c in std::fs::read_dir(crates()).expect("crates/").flatten() {
        let c = c.path();
        if c.file_name().is_some_and(|n| n == "devdeck-sys") {
            continue;
        }
        for sub in ["src", "tests", "benches"] {
            walk(&c.join(sub), &mut out);
        }
        if c.join("build.rs").is_file() {
            out.push(c.join("build.rs"));
        }
    }
    out
}

/// What a file names of [`FORBIDDEN`], comment lines left out.
fn named(path: &Path) -> Vec<(usize, &'static str)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut found = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        found.extend(FORBIDDEN.iter().filter(|f| line.contains(*f)).map(|f| (i + 1, *f)));
    }
    found
}

#[test]
fn the_os_is_reached_only_through_devdeck_sys() {
    let root = crates();
    let mut wrong = Vec::new();
    let mut moved = Vec::new();
    for path in sources() {
        let rel = path.strip_prefix(&root).unwrap().to_string_lossy().into_owned();
        let found = named(&path);
        let listed = NOT_YET.iter().any(|(f, _)| *f == rel);
        if !found.is_empty() && !listed {
            wrong.extend(found.into_iter().map(|(line, what)| format!("{rel}:{line}: {what}")));
        } else if found.is_empty() && listed {
            moved.push(rel);
        }
    }
    assert!(wrong.is_empty(), "OS code outside devdeck-sys:\n{}", wrong.join("\n"));
    assert!(moved.is_empty(), "no longer OS-specific, take off NOT_YET: {moved:?}");
    for (f, _) in NOT_YET {
        assert!(root.join(f).is_file(), "NOT_YET names a file that isn't there: {f}");
    }
}

/// The dependency names of a `Cargo.toml`: the keys of its `[dependencies]`,
/// `[dev-dependencies]`, `[build-dependencies]` and `[target.….dependencies]` tables.
fn dependencies(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_deps = line.trim_matches(['[', ']']).ends_with("dependencies");
        } else if in_deps && !line.is_empty() && !line.starts_with('#') {
            let key = line.split(['=', '.']).next().unwrap_or_default().trim();
            out.push(key.trim_matches('"').to_string());
        }
    }
    out
}

#[test]
fn only_devdeck_sys_depends_on_the_crates_that_call_the_os() {
    let mut wrong = Vec::new();
    for c in std::fs::read_dir(crates()).expect("crates/").flatten() {
        let manifest = c.path().join("Cargo.toml");
        if c.file_name() == "devdeck-sys" || !manifest.is_file() {
            continue;
        }
        let deps = dependencies(&std::fs::read_to_string(&manifest).unwrap());
        wrong.extend(deps.into_iter().filter(|d| OS_CRATES.contains(&d.as_str())).map(|d| format!("{}: {d}", c.file_name().to_string_lossy())));
    }
    assert!(wrong.is_empty(), "OS crates outside devdeck-sys: {wrong:?}");
}

#[test]
fn dependency_keys_are_read_from_dependency_tables_only() {
    let manifest = "[package]\nname = \"x\"\n# libc\n[dependencies]\nserde.workspace = true\nlibc = \"0.2\"\n\n[features]\nunix = []\n[target.'cfg(unix)'.dev-dependencies]\nnix = { version = \"1\" }\n";
    assert_eq!(dependencies(manifest), ["serde", "libc", "nix"]);
}
