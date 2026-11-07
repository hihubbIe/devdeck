//! Discovery on fixture trees built in the temp folder (never the user's folders).

use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

/// A folder tree in the temp folder, removed when dropped.
struct Tree(crate::testing::TempDir);

impl Tree {
    fn new(name: &str) -> Tree {
        Tree(crate::testing::TempDir::new(name))
    }

    fn file(&self, rel: &str, text: &str) -> &Tree {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        self
    }

    fn dir(&self, rel: &str) -> &Tree {
        fs::create_dir_all(self.0.join(rel)).unwrap();
        self
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    fn scan(&self) -> Found {
        scan(&[self.0.to_path_buf()], None)
    }
}

fn app<'a>(found: &'a Found, id: &str) -> &'a App {
    found.apps.iter().find(|a| a.id == id).unwrap_or_else(|| panic!("no app {id} in {:?}", ids(found)))
}

fn ids(found: &Found) -> Vec<&str> {
    found.apps.iter().map(|a| a.id.as_str()).collect()
}

/// `id → line` of every command, in order.
fn lines(app: &App) -> Vec<(&str, &str)> {
    app.commands.iter().map(|c| (c.id.as_str(), c.line.as_str())).collect()
}

fn tier(app: &App, id: &str) -> Tier {
    app.command(id).unwrap_or_else(|| panic!("no command {id} in {:?}", lines(app))).tier
}

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
    assert_eq!((web.node.clone(), web.env.clone(), web.urls.clone()), (None, EnvApply::None, vec![]));
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
    assert_eq!(site.node.as_deref(), Some("22"));
    assert_eq!(app(&found, "front-end/apps/admin").node.as_deref(), Some("20"), "the root's .nvmrc");
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
fn taskfile_and_makefile_commands_hide_the_bare_go_run() {
    let t = Tree::new("precedence");
    let taskfile = "\
version: '3'
tasks:
  run:
    cmds:
      - echo replaced by the duplicate below
  build: go build ./...
  lint:
    - golangci-lint run
  gen:
    internal: true
    cmds: [go generate ./...]
  run:
    desc: serve
    cmds:
      - cmd: go run main.go serve -c configs
";
    t.file("svc/go.mod", "module svc\n").file("svc/main.go", "// Command svc.\npackage main\n\nfunc main() {}\n");
    t.file("svc/main_test.go", "package main_test\n").file("svc/Taskfile.yml", taskfile);
    t.file("api/go.mod", "module api\n")
        .file("api/cmd/api/main.go", "/* build\n tags */\npackage main\n")
        .file("api/cmd/worker/main.go", "package main\n")
        .file("api/cmd/shared/util.go", "package shared\n")
        .file("api/Makefile", ".PHONY: run\nrun:\n\tgo run ./cmd/api -p 1\n");
    t.file("golib/go.mod", "module golib\n").file("golib/lib.go", "// Package golib.\npackage golib\n");
    // A generator kept next to the library: excluded from builds, not a main.
    t.file("golib/gen.go", "//go:build ignore\n\npackage main\n\nfunc main() {}\n");
    let found = t.scan();
    assert!(found.problems.is_empty(), "{:?}", found.problems);

    let svc = app(&found, "svc");
    assert_eq!((svc.kind.as_str(), svc.library), ("go", false));
    assert_eq!(
        lines(svc),
        [
            ("go:build", "go build ./..."),
            ("go:run", "go run ."),
            ("go:test", "go test ./..."),
            ("task:build", "task build"),
            ("task:lint", "task lint"),
            ("task:run", "task run"),
        ],
        "internal tasks left out; duplicate keys tolerated"
    );
    assert_eq!(tier(svc, "go:run"), Tier::Hidden, "the Taskfile runs it");
    assert_eq!(tier(svc, "task:run"), Tier::Primary);
    assert_eq!(svc.command("task:run").unwrap().tools, ["task"]);
    assert_eq!(svc.command("go:run").unwrap().tools, ["go"]);

    let api = app(&found, "api");
    assert_eq!(
        lines(api),
        [
            ("go:api", "go run ./cmd/api"),
            ("go:build", "go build ./..."),
            ("go:test", "go test ./..."),
            ("go:worker", "go run ./cmd/worker"),
            ("make:run", "make run"),
        ]
    );
    assert_eq!(tier(api, "go:api"), Tier::Hidden, "the Makefile runs it");
    assert_eq!(tier(api, "go:worker"), Tier::Secondary);
    assert!(!api.library);
    assert!(found.watch.contains(&t.path("api/cmd")));

    let golib = app(&found, "golib");
    assert!(golib.library, "no main package");
    assert_eq!(lines(golib), [("go:build", "go build ./..."), ("go:test", "go test ./...")]);
}

#[test]
fn compose_files_give_up_and_down() {
    let t = Tree::new("compose");
    t.file("stack/docker-compose.yml", "services:\n  db:\n    image: postgres\n")
        .file("stack/docker-compose.override.yml", "services: {}\n")
        .file("stack/docker-compose.dev.yml", "services: {}\n")
        .file("stack/compose.broken.yaml", "services: [\n");
    let found = t.scan();
    let stack = app(&found, "stack");
    assert_eq!((stack.kind.as_str(), stack.library), ("docker", false));
    assert_eq!(
        lines(stack),
        [
            ("compose:down", "docker compose down"),
            ("compose:down:dev", "docker compose -f docker-compose.dev.yml down"),
            ("compose:up", "docker compose up"),
            ("compose:up:dev", "docker compose -f docker-compose.dev.yml up"),
        ]
    );
    assert_eq!(tier(stack, "compose:up:dev"), Tier::Primary);
    assert_eq!(stack.command("compose:up").unwrap().tools, ["docker"]);
    assert_eq!(found.problems.len(), 1, "{:?}", found.problems);
    assert!(found.problems[0].contains("stack/compose.broken.yaml: "), "{:?}", found.problems);
}

#[test]
fn launch_configurations_become_commands() {
    let t = Tree::new("vscode");
    let launch = r#"{
  // VS Code writes comments and trailing commas
  "version": "0.2.0",
  "configurations": [
    {"name": "Server", "type": "go", "request": "launch", "program": "${workspaceFolder}", "args": ["serve"],},
    {"name": "Attach", "type": "go", "request": "attach", "mode": "local"},
    {"name": "Script", "type": "node", "request": "launch", "program": "${workspaceFolder}/index.js"},
    {"name": "Current file", "type": "go", "request": "launch", "program": "${file}"},
    {"name": "Py", "type": "python", "request": "launch", "program": "x.py"},
  ],
}"#;
    t.file("tool/go.mod", "module tool\n").file("tool/main.go", "package main\n").file("tool/.vscode/launch.json", launch);
    t.file("script/.vscode/launch.json", launch);
    t.file("settings/.vscode/settings.json", "{}");
    let found = t.scan();
    assert!(found.problems.is_empty(), "{:?}", found.problems);
    assert_eq!(ids(&found), ["script", "tool"], "a launch.json with commands is enough; settings alone isn't");

    let tool = app(&found, "tool");
    assert_eq!(
        lines(tool),
        [
            ("go:build", "go build ./..."),
            ("go:run", "go run ."),
            ("go:test", "go test ./..."),
            ("vscode:Script", "node ./index.js"),
            ("vscode:Server", "go run . serve"),
        ]
    );
    assert_eq!(tier(tool, "go:run"), Tier::Hidden, "launch.json runs it");
    assert_eq!(tool.command("vscode:Script").unwrap().tools, ["node"]);
    assert_eq!(app(&found, "script").kind, "vscode");
    assert!(found.watch.contains(&t.path("tool/.vscode")));
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
fn broken_manifests_are_problems() {
    let t = Tree::new("broken");
    t.file("a/package.json", "{ nope");
    t.file("b/Taskfile.yml", "tasks: [\n");
    t.file("c/Cargo.toml", "[package\nname = 1\n");
    t.file("d/.vscode/launch.json", "{ [");
    t.file("e/pnpm-workspace.yaml", "packages: {\n").file("e/package.json", "{}");
    let found = t.scan();
    assert_eq!(ids(&found), ["a", "b", "c", "e"], "a broken manifest still makes a project; launch.json alone doesn't");
    for id in ["a", "b", "c"] {
        let a = app(&found, id);
        assert!(a.commands.is_empty() && !a.library, "{id}: {a:?}");
    }
    assert_eq!(app(&found, "a").kind, "npm");
    let at = |file: &str| found.problems.iter().find(|p| p.starts_with(&format!("{}: ", t.path(file).display()))).cloned();
    for file in ["a/package.json", "b/Taskfile.yml", "c/Cargo.toml", "d/.vscode/launch.json", "e/pnpm-workspace.yaml"] {
        assert!(at(file).is_some(), "{file}: {:?}", found.problems);
    }
    assert!(at("c/Cargo.toml").unwrap().contains("line 1: "), "{:?}", at("c/Cargo.toml"));
    assert!(found.problems.iter().all(|p| !p.contains('\n')), "one line each: {:?}", found.problems);
}

#[test]
fn the_same_id_in_two_workspace_folders_is_reported() {
    let (one, two) = (Tree::new("dup-one"), Tree::new("dup-two"));
    one.file("x/package.json", "{}");
    two.file("x/Makefile", "run:\n\ttrue\n").file("y/Makefile", "run:\n\ttrue\n");
    let found = scan(&[one.0.to_path_buf(), two.0.to_path_buf()], None);
    assert_eq!(ids(&found), ["x", "y"]);
    assert_eq!(app(&found, "x").dir, one.path("x"), "the first one is kept");
    assert_eq!(found.problems, [format!("{}: `x` is also in {}; skipped", two.path("x").display(), one.0.display())]);
}

#[test]
fn symlink_loops_are_problems() {
    let t = Tree::new("loops");
    symlink("self", t.path("self")).unwrap();
    symlink(&t.0, t.path("up")).unwrap();
    t.file("mono/package.json", "{}").file("mono/pnpm-workspace.yaml", "packages: ['packages/**']\n");
    t.file("mono/packages/a/package.json", "{}");
    symlink("..", t.path("mono/packages/a/back")).unwrap();
    t.file("real/Makefile", "run:\n\ttrue\n");
    symlink("real", t.path("alias")).unwrap();
    let found = t.scan();
    assert_eq!(ids(&found), ["alias", "mono", "mono/packages/a", "real"], "a link to a project is a project");
    let mut problems: Vec<String> = found.problems.iter().map(|p| p.replace(&t.0.display().to_string(), "")).collect();
    problems.sort();
    assert_eq!(problems, ["/mono/packages/a/back: symlink loop", "/self: symlink loop", "/up: symlink loop"]);
}

#[test]
fn unreadable_and_missing_folders_are_problems() {
    let t = Tree::new("unreadable");
    t.file("locked/package.json", "{}").file("open/Makefile", "run:\n\ttrue\n");
    let locked = t.path("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read_dir(&locked).is_ok(); // root reads anything
    let found = scan(&[t.0.to_path_buf(), t.path("missing")], None);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(ids(&found).last(), Some(&"open"));
    assert!(found.problems.iter().any(|p| p.starts_with(&format!("{}: ", t.path("missing").display()))), "{:?}", found.problems);
    if !readable {
        assert!(found.problems.iter().any(|p| p.starts_with(&format!("{}: ", locked.display()))), "{:?}", found.problems);
    }
}

#[test]
fn folders_show_the_home_folder_as_a_tilde() {
    let t = Tree::new("home");
    t.file("x/Makefile", "run:\n\ttrue\n").file("y/package.json", "{");
    let home = t.0.parent().unwrap();
    let found = scan(&[t.0.to_path_buf()], Some(home));
    let name = t.0.file_name().unwrap().to_str().unwrap();
    assert_eq!(app(&found, "x").folder, format!("~/{name}/x"));
    assert!(found.problems[0].starts_with(&format!("~/{name}/y/package.json: ")), "{:?}", found.problems);
    assert_eq!(display(Path::new("/h/u"), Some(Path::new("/h/u"))), "~");
    assert_eq!(display(Path::new("/srv/x"), Some(Path::new("/h/u"))), "/srv/x");
    assert_eq!(display(Path::new("/srv/x"), Some(Path::new("/"))), "/srv/x", "a home of / isn't shortened");
    assert_eq!(display(Path::new("/h/u/x"), None), "/h/u/x");
}

#[test]
fn shell_words_are_quoted_when_needed() {
    assert_eq!(quote("start:dev"), "start:dev");
    assert_eq!(quote("./cmd/api"), "./cmd/api");
    assert_eq!(quote("a b"), "'a b'");
    assert_eq!(quote("it's"), r"'it'\''s'");
    assert_eq!(quote(""), "''");
}

#[test]
fn empty_yaml_gives_defaults() {
    #[derive(Debug, Default, PartialEq, serde::Deserialize)]
    struct Doc {
        #[serde(default)]
        packages: Vec<String>,
    }
    assert_eq!(yaml::<Doc>(""), Ok(Doc::default()));
    assert_eq!(yaml::<Doc>("# only a comment\n"), Ok(Doc::default()));
    assert_eq!(yaml::<Doc>("packages: [a]\npackages: [b]\n"), Ok(Doc { packages: vec!["b".into()] }), "duplicates: the last wins");
}

#[test]
fn discover_reads_the_given_folders() {
    let t = Tree::new("discover");
    t.file("x/Makefile", "run:\n\ttrue\n");
    assert_eq!(ids(&discover(&[t.0.to_path_buf()])), ["x"]);
}

#[test]
fn reads_names_the_files_discovery_reads() {
    for name in ["package.json", "pnpm-workspace.yaml", "Taskfile.yml", "GNUmakefile", "compose.yaml", "docker-compose.dev.yml", "launch.json", "main.go", ".nvmrc"] {
        assert!(reads(name), "{name}");
    }
    for name in ["tsconfig.tsbuildinfo", "debug.log", ".env", "main_test.go", "docker-compose.override.yml", "README.md"] {
        assert!(!reads(name), "{name}");
    }
}

#[test]
fn projects_inside_plain_folders_are_found_down_to_the_depth() {
    let t = Tree::new("nested");
    t.file("vhult/graph/package.json", r#"{"scripts": {"dev": "vite"}}"#);
    t.file("games/2d/tetris/Makefile", "run:\n\ttrue\n");
    t.file("deep/a/b/c/Makefile", "run:\n\ttrue\n");
    t.file("api/Makefile", "run:\n\ttrue\n").file("api/tools/gen/Makefile", "gen:\n\ttrue\n");
    t.dir("empty/nothing/here");
    let found = t.scan();
    assert_eq!(ids(&found), ["api", "games/2d/tetris", "vhult/graph"], "level 4 is too deep; inside a project is the project's");
    assert_eq!(app(&found, "vhult/graph").name, "graph");
    for dir in ["vhult", "games", "games/2d", "empty/nothing"] {
        assert!(found.watch.contains(&t.path(dir)), "{dir} watched: a project added there is seen");
    }
    assert!(!found.watch.contains(&t.path("api/tools")), "a project's inside isn't walked");
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
            ("cargo:run:devdeck-snapshot", "cargo run --bin devdeck-snapshot --features snapshot"),
            ("cargo:run:extra:tool", "cargo run -p extra --bin tool"),
            ("cargo:run:gen", "cargo run --bin gen"),
            ("cargo:run:tool:tool", "cargo run -p tool --bin tool"),
        ]
    );
    assert_eq!(tier(app(&found, "ws"), "cargo:run:devdeck"), Tier::Primary);
    assert_eq!(runs("one"), [("cargo:run:one", "cargo run --bin one --features cli")], "a lone binary needing a feature");
    assert_eq!(runs("chosen"), [("cargo:run", "cargo run")], "default-run chooses");
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
    let deps = |id: &str, cmd: &str| app(&found, id).command(cmd).unwrap().deps.clone();
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
fn an_install_concerns_discovery_and_its_files_dont() {
    let t = Tree::new("concerns");
    t.dir("web/node_modules/.bin").dir("web/node_modules/vite");
    assert!(concerns(&t.path("web/node_modules/.bin")), "the programs folder appears");
    assert!(concerns(&t.path("web/node_modules/.bin/vite")), "a program is installed or removed");
    assert!(!concerns(&t.path("web/node_modules/vite")), "a package's own folder");
    assert!(concerns(&t.path("web/package.json")) && concerns(&t.path("web/node_modules")));
    assert!(!concerns(&t.path("web/debug.log")));
}

#[test]
fn engines_come_from_the_package_or_its_monorepo() {
    let t = Tree::new("engines");
    t.file("graph/package.json", r#"{"engines": {"node": ">=22.0.0"}, "scripts": {"dev": "vite"}}"#);
    t.file("mono/package.json", r#"{"engines": {"node": "^20"}, "workspaces": ["packages/*"]}"#)
        .file("mono/packages/a/package.json", r#"{"scripts": {"dev": "vite"}}"#)
        .file("mono/packages/b/package.json", r#"{"engines": {"node": ">=22"}, "scripts": {"dev": "vite"}}"#);
    let found = t.scan();
    let engines = |id: &str| app(&found, id).engines.clone();
    assert_eq!(engines("graph").as_deref(), Some(">=22.0.0"));
    assert_eq!(engines("mono/packages/a").as_deref(), Some("^20"), "the root's");
    assert_eq!(engines("mono/packages/b").as_deref(), Some(">=22"), "its own first");
}

#[test]
fn a_scripts_programs_honour_quotes_redirections_and_keywords() {
    let p = |s: &str| node::programs(s);
    assert_eq!(p("tsc 2>&1 | tee log"), ["tsc", "tee"], "2>&1 isn't a separator");
    assert_eq!(p("vite build >&2 && eslint . &> out.txt"), ["vite", "eslint"]);
    assert_eq!(p("jest --testPathPattern='(a|b)'"), ["jest"], "quoted separators");
    assert_eq!(p(r#"node -e "x && y" && grep -E "a|b" f"#), ["grep"]);
    assert_eq!(p(r#"FOO="a b" BAR=1 vite"#), ["vite"], "quoted assignments");
    assert_eq!(p("if [ -d dist ]; then rimraf dist; fi"), ["rimraf"], "after then");
    assert_eq!(p("ulimit -n 4096; read x; : noop; > log vite"), ["vite"], "builtins and a leading redirection");
    assert_eq!(p("cross-env NODE_ENV=production webpack"), ["cross-env"]);
    assert_eq!(p("time exec tsx watch src"), ["tsx"]);
}
