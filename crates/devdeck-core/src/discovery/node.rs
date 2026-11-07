//! `package.json` scripts, and the packages of pnpm, npm and yarn workspaces.
//!
//! A script's programs that the project's install provides and that aren't installed
//! yet (in no `node_modules/.bin` from the package up) are its command's `deps`: the
//! tool check says to install. A project (not a monorepo's package) with a readable
//! `package.json` gets `<manager>:install` unless a script is named `install`. Its `node_modules` and
//! `node_modules/.bin` are watched, so an install made elsewhere is seen.

use super::{Cx, Listing, Part, quote, yaml};
use crate::apps::Tier;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

/// What a `package.json` gave.
pub(super) struct Node {
    pub part: Part,
    /// `npm`, `pnpm` or `yarn`.
    pub manager: &'static str,
    /// Workspace globs, for a monorepo root.
    pub workspaces: Vec<String>,
    /// `engines.node`.
    pub engines: Option<String>,
}

const MANAGERS: [&str; 3] = ["npm", "pnpm", "yarn"];

/// A script's first word that runs no program an install provides: the shell's own
/// (builtins, keywords) and Node's tools (checked as the command's tools).
const NOT_INSTALLED: [&str; 41] = [
    "cd", "export", "set", "unset", "source", ".", ":", "eval", "fi", "for", "done", "case", "esac", "[", "[[", "}", "test", "true",
    "false", "echo", "printf", "exit", "return", "shift", "wait", "trap", "read", "ulimit", "umask", "pushd", "popd", "declare",
    "local", "alias", "type", "node", "npm", "npx", "pnpm", "yarn", "corepack",
];

/// Words before a command that aren't it: `then foo`, `time foo`, `exec foo`.
const PREFIXES: [&str; 12] = ["if", "then", "else", "elif", "while", "until", "do", "!", "{", "time", "exec", "command"];

/// The programs a script runs: the first word of each command (after keywords, variable
/// assignments and redirections), not the shell's own words, paths (`./build.sh`) nor
/// expansions. Quotes are honoured: `node -e "a && b"` is one command.
pub(super) fn programs(script: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for words in commands(script) {
        let mut words = words.iter().map(String::as_str).peekable();
        let program = loop {
            let Some(w) = words.next() else { break None };
            if PREFIXES.contains(&w) || is_assignment(w) {
                continue;
            }
            if let Some(target) = redirection(w) {
                // `2> log`: the next word is where it goes.
                if target.is_empty() {
                    words.next();
                }
                continue;
            }
            break Some(w);
        };
        let Some(p) = program else { continue };
        if p.is_empty() || p.contains(['/', '$', '`']) || NOT_INSTALLED.contains(&p) || out.iter().any(|o| o == p) {
            continue;
        }
        out.push(p.to_string());
    }
    out
}

/// A script's commands as words, split on `&&`, `||`, `|`, `;`, `&`, newlines and
/// parentheses outside quotes; quotes removed. `>&` and `&>` are redirections, not
/// separators.
fn commands(script: &str) -> Vec<Vec<String>> {
    let (mut all, mut words, mut word) = (Vec::new(), Vec::new(), String::new());
    let mut quote: Option<char> = None;
    let chars: Vec<char> = script.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                i += 1;
                word.extend(chars.get(i));
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (None, '\\') => {
                i += 1;
                word.extend(chars.get(i));
            }
            (None, '&') if word.ends_with(['>', '<']) || chars.get(i + 1) == Some(&'>') => word.push(c),
            (None, '&' | '|' | ';' | '\n' | '(' | ')') => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                if !words.is_empty() {
                    all.push(std::mem::take(&mut words));
                }
            }
            (None, c) if c.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            (None, c) => word.push(c),
        }
        i += 1;
    }
    if !word.is_empty() {
        words.push(word);
    }
    if !words.is_empty() {
        all.push(words);
    }
    all
}

/// `NAME=value` before a command.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// For a redirection (`>log`, `2>&1`, `&>`, `<in`, `>>`), what follows its operator
/// (empty when the target is the next word).
fn redirection(word: &str) -> Option<&str> {
    let rest = word.trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = rest.strip_prefix('&').filter(|r| r.starts_with('>')).unwrap_or(rest);
    if !rest.starts_with(['>', '<']) {
        return None;
    }
    Some(rest.trim_start_matches(['>', '<', '&']))
}

/// The scripts of `dir/package.json`, run with the `packageManager` field's manager,
/// else `inherited` (a package gets its monorepo root's), else the lockfile's, else npm.
/// Workspaces are read when nothing is inherited (packages don't nest).
pub(super) fn scan(dir: &Path, listing: &Listing, inherited: Option<&'static str>, cx: &mut Cx) -> Option<Node> {
    if !listing.file("package.json") {
        return None;
    }
    let path = dir.join("package.json");
    let json = cx.read(&path).and_then(|text| match serde_json::from_str::<Value>(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            cx.problem(&path, e);
            None
        }
    });
    let ok = json.is_some();
    let json = json.unwrap_or(Value::Null);
    let field = json.get("packageManager").and_then(Value::as_str).and_then(|s| {
        let name = s.split('@').next().unwrap_or_default();
        MANAGERS.into_iter().find(|m| *m == name)
    });
    let manager = field.or(inherited).unwrap_or_else(|| lockfile(listing));

    let scripts: Vec<(&str, &str)> = json
        .get("scripts")
        .and_then(Value::as_object)
        .map(|o| o.iter().filter_map(|(k, v)| Some((k.as_str(), v.as_str()?))).collect())
        .unwrap_or_default();
    let names: Vec<&str> = scripts.iter().map(|(n, _)| *n).collect();
    let mut part = Part::new(manager);
    for (name, body) in &scripts {
        let line = match manager {
            "yarn" => format!("yarn {}", quote(name)),
            m => format!("{m} run {}", quote(name)),
        };
        part.add(manager, name, line, &names, &[manager, "node"]);
        let deps: Vec<String> = programs(body).into_iter().filter(|p| !cx.installed(dir, p)).collect();
        part.commands.last_mut().expect("just added").deps = deps;
        part.runs.push(body.to_string());
    }
    if ok && inherited.is_none() && !names.contains(&"install") {
        part.add(manager, "install", format!("{manager} install"), &names, &[manager, "node"]);
    }
    cx.watch(&dir.join("node_modules"));
    cx.watch(&dir.join("node_modules/.bin"));
    part.runnable = ok.then(|| part.commands.iter().any(|c| c.tier == Tier::Primary));
    let workspaces = if inherited.is_none() { workspaces(dir, listing, &json, cx) } else { Vec::new() };
    let engines = json.get("engines").and_then(|e| e.get("node")).and_then(Value::as_str).map(str::to_string);
    Some(Node { part, manager, workspaces, engines })
}

/// The manager a lockfile names.
fn lockfile(listing: &Listing) -> &'static str {
    if listing.file("pnpm-lock.yaml") || listing.file("pnpm-workspace.yaml") {
        "pnpm"
    } else if listing.file("yarn.lock") {
        "yarn"
    } else {
        "npm"
    }
}

/// Globs from `pnpm-workspace.yaml`, else from the `workspaces` field (a list, or yarn's
/// `{ packages: [...] }`).
fn workspaces(dir: &Path, listing: &Listing, json: &Value, cx: &mut Cx) -> Vec<String> {
    #[derive(Default, Deserialize)]
    struct PnpmWorkspace {
        #[serde(default)]
        packages: Vec<String>,
    }
    if listing.file("pnpm-workspace.yaml") {
        let path = dir.join("pnpm-workspace.yaml");
        let Some(text) = cx.read(&path) else { return Vec::new() };
        return match yaml::<PnpmWorkspace>(&text) {
            Ok(w) => w.packages,
            Err(e) => {
                cx.problem(&path, e);
                Vec::new()
            }
        };
    }
    let list = match json.get("workspaces") {
        Some(Value::Array(a)) => a,
        Some(Value::Object(o)) => match o.get("packages") {
            Some(Value::Array(a)) => a,
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    list.iter().filter_map(Value::as_str).map(String::from).collect()
}
