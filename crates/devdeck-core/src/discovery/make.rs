//! Makefile targets (`make:`). Included makefiles are not followed; file and pattern
//! targets (`bin/app`, `%.o`) and special ones (`.PHONY`) are left out.

use super::{Cx, Listing, Part, quote};
use std::collections::BTreeMap;
use std::path::Path;

/// The names `make` looks for, in its order.
const NAMES: [&str; 3] = ["GNUmakefile", "makefile", "Makefile"];

/// Lines that start with these are directives, not rules.
const DIRECTIVES: [&str; 9] = ["include", "-include", "sinclude", "export", "unexport", "override", "vpath", "undefine", "private"];
/// Conditionals can sit between a rule and its recipe lines.
const CONDITIONALS: [&str; 6] = ["ifeq", "ifneq", "ifdef", "ifndef", "else", "endif"];

pub(super) fn scan(dir: &Path, listing: &Listing, cx: &mut Cx) -> Option<Part> {
    let name = NAMES.into_iter().find(|n| listing.file(n))?;
    let mut part = Part::new("make");
    let Some(text) = cx.read(&dir.join(name)) else { return Some(part) };
    let targets = targets(&text);
    let names: Vec<&str> = targets.keys().map(String::as_str).collect();
    for (target, recipe) in &targets {
        part.add("make", target, format!("make {}", quote(target)), &names, &["make"]);
        part.runs.push(recipe.clone());
    }
    Some(part)
}

/// The targets of a Makefile with their recipe lines.
fn targets(text: &str) -> BTreeMap<String, String> {
    let text = text.replace("\\\r\n", " ").replace("\\\n", " ");
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    let mut current: Vec<String> = Vec::new();
    let mut in_define = false;
    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        if in_define {
            in_define = raw.split_whitespace().next() != Some("endef");
            continue;
        }
        if let Some(recipe) = raw.strip_prefix('\t') {
            for t in &current {
                let body = out.entry(t.clone()).or_default();
                body.push_str(recipe);
                body.push('\n');
            }
            continue;
        }
        let line = raw.split('#').next().unwrap_or_default();
        let Some(word) = line.split_whitespace().next() else { continue };
        if CONDITIONALS.contains(&word) {
            continue;
        }
        current.clear();
        if word == "define" {
            in_define = true;
            continue;
        }
        if DIRECTIVES.contains(&word) {
            continue;
        }
        let Some(colon) = rule_colon(line) else { continue };
        let rest = line[colon + 1..].trim_start_matches(':');
        let (prerequisites, recipe) = rest.split_once(';').unwrap_or((rest, ""));
        if prerequisites.contains('=') {
            // A target-specific variable (`build: CGO_ENABLED = 0`).
            continue;
        }
        for target in line[..colon].split_whitespace().filter(|t| plain(t)) {
            let body = out.entry(target.to_string()).or_default();
            if !recipe.trim().is_empty() {
                body.push_str(recipe.trim());
                body.push('\n');
            }
            current.push(target.to_string());
        }
    }
    out
}

/// Where a rule's colon is, or `None` for an assignment (`A = b:c`, `A := b`, `A ::= b`)
/// or a line without one.
fn rule_colon(line: &str) -> Option<usize> {
    let colon = line.find(':')?;
    if line[..colon].contains('=') {
        return None;
    }
    let after = line[colon + 1..].trim_start_matches(':');
    (!after.starts_with('=')).then_some(colon)
}

/// A target someone would run by name.
fn plain(target: &str) -> bool {
    !target.starts_with('.') && !target.contains(['/', '%', '$', '(', ')'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_give_targets_and_recipes() {
        let text = "\
GO ?= go
BIN := bin/app
URL = http://x:80
.PHONY: run build
# comment: not a rule
run: build ## start it
\t$(GO) run . serve \\
\t  -c configs
build test:
\tgo build ./...
build: CGO_ENABLED = 0
bin/app: main.go
\tgo build -o $@
%.o: %.c
\tcc $<
define HELP
usage: make run
endef
ifdef DEBUG
lint: ; golangci-lint run
endif
";
        let t = targets(text);
        assert_eq!(t.keys().collect::<Vec<_>>(), ["build", "lint", "run", "test"]);
        assert!(t["run"].starts_with("$(GO) run . serve ") && t["run"].ends_with("-c configs\n"), "continued: {:?}", t["run"]);
        assert_eq!(t["build"], "go build ./...\n");
        assert_eq!(t["lint"], "golangci-lint run\n");
    }
}
