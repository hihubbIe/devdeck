//! The OS stays here (R11.6, `docs/PLATFORMS.md` §4): no source file of another crate
//! names Linux's files, its runtime folder, systemd, or the crates that call the OS.
//! Line comments are skipped. Manifests guard the rest once a crate has nothing left to
//! move: `devdeck-app` dropped `rustix` and `libc` at step 1c, `devdeck-core` does at 1d.

use std::path::{Path, PathBuf};

const FORBIDDEN: &[&str] = &["/proc", "/sys/", "XDG_RUNTIME_DIR", "systemd-run", "std::os::linux", "rustix::", "libc::", "signal_hook"];

/// Files still to move, each with the step of `docs/plans/macos.md` that moves it. The
/// list empties as they move; a file that no longer names any of the above must leave it.
const NOT_YET: &[(&str, &str)] = &[
    ("devdeck-core/src/process.rs", "1d"),
    ("devdeck-core/src/scope.rs", "1d"),
    ("devdeck-core/src/git.rs", "1d"),
    ("devdeck-core/src/environment.rs", "1d"),
];

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
