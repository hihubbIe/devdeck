//! `package.json` scripts, and the packages of pnpm, npm and yarn workspaces.

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
}

const MANAGERS: [&str; 3] = ["npm", "pnpm", "yarn"];

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
        part.runs.push(body.to_string());
    }
    part.runnable = ok.then(|| part.commands.iter().any(|c| c.tier == Tier::Primary));
    let workspaces = if inherited.is_none() { workspaces(dir, listing, &json, cx) } else { Vec::new() };
    Some(Node { part, manager, workspaces })
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
