//! Go modules: `go:run` for a main package at the root, `go:<x>` for `cmd/<x>`, plus
//! `go:build` and `go:test`. A module without a main package is a library.

use super::{Cx, Listing, Part, quote};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub(super) const KIND: &str = "go";

pub(super) fn scan(dir: &Path, listing: &Listing, cx: &mut Cx) -> Option<Part> {
    if !listing.file("go.mod") {
        return None;
    }
    let root = listing.files().any(|f| is_main(&dir.join(f)));
    let mut mains: Vec<String> = Vec::new();
    if listing.dir("cmd") {
        let cmd = dir.join("cmd");
        cx.watch(&cmd);
        if let Some(l) = cx.listing(&cmd) {
            for name in l.folders() {
                if Listing::read(&cmd.join(name)).is_ok_and(|l| l.files().any(|f| is_main(&cmd.join(name).join(f)))) {
                    mains.push(name.to_string());
                }
            }
        }
    }
    let mut commands: Vec<(&str, String)> = Vec::new();
    if root {
        commands.push(("run", "go run .".into()));
    }
    commands.push(("build", "go build ./...".into()));
    commands.push(("test", "go test ./...".into()));
    for m in &mains {
        commands.push((m, format!("go run {}", quote(&format!("./cmd/{m}")))));
    }
    let names: Vec<&str> = commands.iter().map(|(n, _)| *n).collect();
    let mut part = Part::new(KIND);
    for (name, line) in &commands {
        part.add(KIND, name, line.clone(), &names, &["go"]);
    }
    part.runnable = Some(root || !mains.is_empty());
    Some(part)
}

/// A Go source file (not a test) of package `main`.
fn is_main(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    name.ends_with(".go") && !name.ends_with("_test.go") && package_of(path).as_deref() == Some("main")
}

/// The package a Go file declares, read up to its `package` clause.
fn package_of(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut in_comment = false;
    for line in BufReader::new(file).lines() {
        let line = line.ok()?;
        let mut rest = line.trim();
        loop {
            if in_comment {
                let Some(end) = rest.find("*/") else {
                    rest = "";
                    break;
                };
                rest = rest[end + 2..].trim_start();
                in_comment = false;
            } else if let Some(r) = rest.strip_prefix("/*") {
                rest = r;
                in_comment = true;
            } else {
                break;
            }
        }
        if rest.is_empty() || rest.starts_with("//") {
            continue;
        }
        let name = rest.strip_prefix("package")?.split(|c: char| c.is_whitespace() || c == ';' || c == '/').find(|s| !s.is_empty())?;
        return Some(name.to_string());
    }
    None
}

/// The Go command ids a shell text runs with `go run`: `go:run` for the root package
/// (`go run .`, `go run -race main.go`), `go:<x>` for `./cmd/<x>`.
pub(super) fn targets(text: &str) -> Vec<String> {
    /// Build flags followed by a value when written without `=`.
    const VALUED: [&str; 12] = ["-tags", "-ldflags", "-gcflags", "-asmflags", "-mod", "-modfile", "-exec", "-p", "-pkgdir", "-overlay", "-o", "-C"];
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || ";&|()`".contains(c))
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if *w != "go" || words.get(i + 1) != Some(&"run") {
            continue;
        }
        let mut j = i + 2;
        while let Some(flag) = words.get(j).filter(|w| w.starts_with('-')) {
            j += if VALUED.contains(flag) { 2 } else { 1 };
        }
        let Some(target) = words.get(j) else { continue };
        let target = target.trim_matches(['"', '\'']);
        let target = target.strip_prefix("./").unwrap_or(target).trim_end_matches('/');
        // A file names its folder's package.
        let package = if target.ends_with(".go") { target.rsplit_once('/').map_or("", |(d, _)| d) } else { target };
        match package {
            "" | "." => out.push(format!("{KIND}:run")),
            p => {
                if let Some(x) = p.strip_prefix("cmd/").filter(|x| !x.is_empty() && !x.contains('/')) {
                    out.push(format!("{KIND}:{x}"));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_run_targets_are_found_in_shell_text() {
        assert_eq!(targets("go run main.go serve -c configs"), ["go:run"]);
        assert_eq!(targets("CGO_ENABLED=0 go run -race -tags it . serve"), ["go:run"]);
        assert_eq!(targets("cd x && go run ./cmd/api/ -p 1"), ["go:api"]);
        assert_eq!(targets("go run ./cmd/api/main.go"), ["go:api"]);
        assert_eq!(targets("go run ./internal/tool"), Vec::<String>::new());
        assert_eq!(targets("go build ./... && go test ./..."), Vec::<String>::new());
        assert_eq!(targets("go run github.com/x/y@latest"), Vec::<String>::new());
    }
}
