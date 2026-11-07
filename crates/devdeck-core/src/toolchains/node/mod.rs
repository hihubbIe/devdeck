//! Node: `package.json` scripts (`npm:`, `pnpm:` or `yarn:` from the `packageManager`
//! field, else the lockfile), and the packages of pnpm, npm and yarn workspaces (a
//! package runs with its monorepo root's manager).
//!
//! - Which Node an app runs with: [`versions`] (`.nvmrc`, then `engines.node`).
//! - A script's programs that the project's install provides (`vite`, `tsc`) and that
//!   aren't installed (in no `node_modules/.bin` from the package up: [`installed`])
//!   are its command's `from_install`: running it says to install. A project (not a
//!   monorepo's package) with a readable `package.json` gets `<manager>:install` (a
//!   setup command) unless a script is named `install`. Its `node_modules` and
//!   `node_modules/.bin` are watched, so an install made elsewhere is seen.

mod installed;
pub mod semver;
pub mod versions;

use super::{Cx, Listing, Part, Toolchain, quote, shell, yaml};
use crate::apps::{App, Command, Tier};
use crate::environment::which;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub use versions::Machine;

/// What Node keeps about an app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// `npm`, `pnpm` or `yarn`, when it has a `package.json`.
    pub manager: Option<&'static str>,
    /// What its `.nvmrc` asks for (R6.8).
    pub nvmrc: Option<String>,
    /// The Node versions its `package.json` accepts (`engines.node`, a semver range like
    /// `>=22`); a monorepo's packages share the root's.
    pub engines: Option<String>,
}

pub struct Node;

const MANAGERS: [&str; 3] = ["npm", "pnpm", "yarn"];

/// Programs a Node install provides: looked up with the app's Node first.
const PROGRAMS: [&str; 6] = ["node", "npm", "npx", "pnpm", "yarn", "corepack"];

impl Toolchain for Node {
    fn sources(&self) -> &'static [&'static str] {
        &MANAGERS
    }

    fn reads(&self, name: &str) -> bool {
        ["package.json", "pnpm-lock.yaml", "pnpm-workspace.yaml", "yarn.lock", ".nvmrc"].contains(&name)
    }

    /// `node_modules/.bin` appearing, or the programs in it changing (an install).
    fn concerns(&self, path: &Path) -> bool {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        match path.parent().and_then(Path::file_name).and_then(|n| n.to_str()) {
            Some("node_modules") => name == ".bin",
            Some(".bin") => true,
            _ => false,
        }
    }

    fn skips(&self, name: &str) -> bool {
        name == "node_modules"
    }

    fn scan(&self, dir: &Path, listing: &Listing, parent: Option<&super::Facts>, cx: &mut Cx) -> Option<Part> {
        scan(dir, listing, parent.and_then(|f| f.node.as_ref()), cx)
    }

    fn is_package(&self, listing: &Listing) -> bool {
        listing.file("package.json")
    }

    /// Any app's `.nvmrc`, `package.json` or not: a Taskfile project gets its Node too.
    fn complete(&self, facts: &mut super::Facts, dir: &Path, cx: &mut Cx) {
        if let Some(v) = versions::nvmrc(dir, cx.workspace()) {
            facts.node.get_or_insert_with(Facts::default).nvmrc = Some(v);
        }
    }

    fn provides(&self, program: &str) -> bool {
        PROGRAMS.contains(&program)
    }

    fn front(&self, app: &App, machine: &super::Machine) -> Result<Vec<PathBuf>, String> {
        let Some(facts) = &app.facts.node else { return Ok(Vec::new()) };
        Ok(machine.node.choose(facts)?.map(|i| i.bin.clone()).into_iter().collect())
    }

    /// npm passes a script's arguments after `--`; pnpm and yarn take them as they are.
    fn with_args(&self, cmd: &Command, line: &str, args: &str) -> String {
        if cmd.source() == "npm" && !cmd.setup { super::after_separator(line, args) } else { format!("{line} {args}") }
    }

    fn installed(&self, _app: &App, cmd: &Command, path: &[PathBuf]) -> Option<String> {
        let missing = cmd.from_install.iter().find(|p| which(p, path).is_none())?;
        Some(format!("{missing} isn't installed: run {} install", cmd.source()))
    }
}

/// The scripts of `dir/package.json`, run with the `packageManager` field's manager,
/// else the monorepo root's (`parent`), else the lockfile's, else npm. Workspaces are
/// read for a root only (packages don't nest).
fn scan(dir: &Path, listing: &Listing, parent: Option<&Facts>, cx: &mut Cx) -> Option<Part> {
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
    let inherited = parent.and_then(|p| p.manager);
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
        let from_install = shell::programs(body).into_iter().filter(|p| !PROGRAMS.contains(&p.as_str()) && !installed::has(dir, p, cx)).collect();
        part.commands.last_mut().expect("just added").from_install = from_install;
        part.runs.push(body.to_string());
    }
    if ok && parent.is_none() && !names.contains(&"install") {
        part.add(manager, "install", format!("{manager} install"), &names, &[manager, "node"]);
        part.commands.last_mut().expect("just added").setup = true;
    }
    cx.watch(&dir.join("node_modules"));
    cx.watch(&dir.join("node_modules/.bin"));
    part.runnable = ok.then(|| part.commands.iter().any(|c| c.tier == Tier::Primary));
    if parent.is_none() {
        part.packages = workspaces(dir, listing, &json, cx);
    }
    let engines = json.get("engines").and_then(|e| e.get("node")).and_then(Value::as_str).map(str::to_string);
    let engines = engines.or_else(|| parent.and_then(|p| p.engines.clone()));
    part.facts.node = Some(Facts { manager: Some(manager), nvmrc: None, engines });
    Some(part)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::EnvApply;
    use crate::testing::{Tree, app, ids, lines, tier};

    #[test]
    fn package_scripts_run_with_the_projects_manager() {
        let t = Tree::new("node");
        let scripts = r#"{"scripts": {"dev": "vite", "build": "vite build", "prebuild": "rm -rf dist", "preview": "vite preview", "lint": "eslint ."}}"#;
        t.file("web/package.json", scripts).file("web/pnpm-lock.yaml", "");
        t.file("shop/package.json", r#"{"scripts": {"start": "node ."}}"#).file("shop/yarn.lock", "");
        t.file("field/package.json", r#"{"packageManager": "pnpm@9.1.0", "scripts": {"serve": "x"}}"#).file("field/package-lock.json", "{}");
        t.file("plain/package.json", r#"{"scripts": {"start": "node .", "count": 3}}"#);
        t.file("utils/package.json", r#"{"name": "utils", "scripts": {"build": "tsc", "test": "jest"}}"#);
        t.file(".hidden/package.json", "{}").file("node_modules/dep/package.json", "{}").dir("empty").file("notes.md", "");
        let found = t.scan();
        assert_eq!(ids(&found), ["field", "plain", "shop", "utils", "web"], "no manifest, hidden, node_modules: not apps");
        assert!(found.problems.is_empty(), "{:?}", found.problems);

        let web = app(&found, "web");
        assert_eq!((web.name.as_str(), web.kind.as_str(), web.library), ("web", "pnpm", false));
        assert_eq!((web.dir.clone(), web.folder.clone()), (t.path("web"), t.path("web").display().to_string()));
        assert_eq!((web.facts.node.clone().unwrap().nvmrc, web.env.clone(), web.urls.clone()), (None, EnvApply::None, vec![]));
        assert_eq!(
            lines(web),
            [
                ("pnpm:build", "pnpm run build"),
                ("pnpm:dev", "pnpm run dev"),
                ("pnpm:install", "pnpm install"),
                ("pnpm:lint", "pnpm run lint"),
                ("pnpm:prebuild", "pnpm run prebuild"),
                ("pnpm:preview", "pnpm run preview"),
            ]
        );
        assert_eq!(web.commands[0].tools, ["pnpm", "node"]);
        assert_eq!(tier(web, "pnpm:install"), Tier::Secondary);
        assert!(web.commands.iter().all(|c| c.ports.is_empty()));
        assert_eq!(tier(web, "pnpm:dev"), Tier::Primary);
        assert_eq!(tier(web, "pnpm:prebuild"), Tier::Hidden, "a hook of build");
        assert_eq!(tier(web, "pnpm:preview"), Tier::Secondary, "no `view` script");
        assert_eq!(tier(web, "pnpm:lint"), Tier::Hidden);

        assert_eq!(lines(app(&found, "shop")), [("yarn:install", "yarn install"), ("yarn:start", "yarn start")]);
        assert_eq!(app(&found, "shop").commands[0].tools, ["yarn", "node"]);
        assert_eq!(lines(app(&found, "field")), [("pnpm:install", "pnpm install"), ("pnpm:serve", "pnpm run serve")], "the packageManager field wins");
        assert_eq!(lines(app(&found, "plain")), [("npm:install", "npm install"), ("npm:start", "npm run start")], "npm by default; non-string scripts skipped");
        assert!(app(&found, "utils").library, "no primary script");
    }

    #[test]
    fn monorepos_expand_into_packages() {
        let t = Tree::new("monorepo");
        t.file("front-end/package.json", r#"{"scripts": {"lint": "eslint ."}}"#)
            .file("front-end/pnpm-lock.yaml", "")
            .file("front-end/.nvmrc", "20\n")
            .file("front-end/pnpm-workspace.yaml", "packages:\n  - 'apps/*'\n  - packages/**\n  - '!packages/legacy'\n");
        t.file("front-end/apps/site/package.json", r#"{"scripts": {"dev": "next dev"}}"#)
            .file("front-end/apps/site/.nvmrc", "22\n")
            .file("front-end/apps/admin/package.json", r#"{"scripts": {"start:dev": "vite"}}"#)
            .file("front-end/apps/.cache/package.json", "{}")
            .dir("front-end/apps/notes");
        t.file("front-end/packages/ui/package.json", r#"{"scripts": {"build": "tsc"}}"#)
            .file("front-end/packages/nested/deep/package.json", "{}")
            .file("front-end/packages/node_modules/dep/package.json", "{}")
            .file("front-end/packages/legacy/package.json", "{}");
        t.file("mono/package.json", r#"{"workspaces": {"packages": ["libs/*"]}}"#)
            .file("mono/yarn.lock", "")
            .file("mono/libs/a/package.json", r#"{"scripts": {"start": "node ."}}"#);
        let found = t.scan();
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        assert_eq!(
            ids(&found),
            [
                "front-end",
                "front-end/apps/admin",
                "front-end/apps/site",
                "front-end/packages/nested/deep",
                "front-end/packages/ui",
                "mono",
                "mono/libs/a",
            ]
        );
        let site = app(&found, "front-end/apps/site");
        assert_eq!((site.name.as_str(), site.kind.as_str(), site.dir.clone()), ("site", "pnpm", t.path("front-end/apps/site")));
        assert_eq!(lines(site), [("pnpm:dev", "pnpm run dev")], "the root's manager");
        assert_eq!(site.facts.node.as_ref().and_then(|f| f.nvmrc.as_deref()), Some("22"));
        assert_eq!(app(&found, "front-end/apps/admin").facts.node.as_ref().and_then(|f| f.nvmrc.as_deref()), Some("20"), "the root's .nvmrc");
        assert!(app(&found, "front-end").library, "the root stays an app; only a hidden script");
        assert!(app(&found, "front-end/packages/ui").library);
        assert_eq!(lines(app(&found, "mono/libs/a")), [("yarn:start", "yarn start")]);

        for dir in ["", "front-end", "front-end/apps", "front-end/apps/site", "front-end/packages", "front-end/packages/ui", "mono/libs"] {
            assert!(found.watch.contains(&t.path(dir)), "watch {dir:?}: {:?}", found.watch);
        }
        assert!(!found.watch.contains(&t.path("front-end/packages/nested")), "not every folder `**` walks");
        assert!(found.watch.windows(2).all(|w| w[0] < w[1]), "sorted, no repeats");
    }

    #[test]
    fn programs_a_script_runs_are_checked_against_the_install() {
        let t = Tree::new("deps");
        let scripts = r#"{"scripts": {"dev": "NODE_ENV=dev vite --port 3000", "build": "rimraf dist && tsc -b && vite build", "lint": "eslint . | tee out.txt", "gen": "./gen.sh; node x.js; echo done", "install": "make"}}"#;
        t.file("web/package.json", scripts);
        t.file("ready/package.json", scripts).file("ready/node_modules/.bin/vite", "").file("ready/node_modules/.bin/eslint", "");
        // A monorepo's packages find what the root installed.
        t.file("mono/package.json", r#"{"workspaces": ["packages/*"]}"#).file("mono/node_modules/.bin/tsc", "");
        t.file("mono/packages/ui/package.json", r#"{"scripts": {"build": "tsc", "story": "storybook dev"}}"#);
        let found = t.scan();
        let deps = |id: &str, cmd: &str| app(&found, id).command(cmd).unwrap().from_install.clone();
        assert_eq!(deps("web", "npm:dev"), ["vite"], "assignments skipped");
        assert_eq!(deps("web", "npm:build"), ["rimraf", "tsc", "vite"], "every command of the script");
        assert_eq!(deps("web", "npm:lint"), ["eslint", "tee"], "pipes too (tee comes from the PATH)");
        assert_eq!(deps("web", "npm:gen"), [] as [&str; 0], "paths, node and the shell's words aren't installed");
        assert_eq!(deps("ready", "npm:dev"), [] as [&str; 0]);
        assert_eq!(deps("ready", "npm:build"), ["rimraf", "tsc"]);
        assert_eq!(deps("mono/packages/ui", "npm:build"), [] as [&str; 0]);
        assert_eq!(deps("mono/packages/ui", "npm:story"), ["storybook"]);
        assert_eq!(app(&found, "web").command("npm:install").unwrap().line, "npm run install", "a script named install is the project's");
        assert!(app(&found, "mono").command("npm:install").is_some());
        assert!(app(&found, "mono/packages/ui").command("npm:install").is_none(), "packages install with their monorepo");
        for dir in ["web/node_modules", "web/node_modules/.bin", "mono/packages/ui/node_modules/.bin"] {
            assert!(found.watch.contains(&t.path(dir)), "{dir} watched: an install is seen");
        }
    }

    #[test]
    fn engines_come_from_the_package_or_its_monorepo() {
        let t = Tree::new("engines");
        t.file("graph/package.json", r#"{"engines": {"node": ">=22.0.0"}, "scripts": {"dev": "vite"}}"#);
        t.file("mono/package.json", r#"{"engines": {"node": "^20"}, "workspaces": ["packages/*"]}"#)
            .file("mono/packages/a/package.json", r#"{"scripts": {"dev": "vite"}}"#)
            .file("mono/packages/b/package.json", r#"{"engines": {"node": ">=22"}, "scripts": {"dev": "vite"}}"#);
        let found = t.scan();
        let engines = |id: &str| app(&found, id).facts.node.as_ref().and_then(|f| f.engines.clone());
        assert_eq!(engines("graph").as_deref(), Some(">=22.0.0"));
        assert_eq!(engines("mono/packages/a").as_deref(), Some("^20"), "the root's");
        assert_eq!(engines("mono/packages/b").as_deref(), Some(">=22"), "its own first");
    }
}
