//! The walk on fixture trees built in the temp folder (never the user's folders):
//! folders, nesting, duplicates, problems, and what crosses toolchains.

use super::*;
use crate::testing::{Tree, app, ids, lines, tier};
use std::os::unix::fs::{PermissionsExt, symlink};

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
fn discover_reads_the_given_folders() {
    let t = Tree::new("discover");
    t.file("x/Makefile", "run:\n\ttrue\n");
    assert_eq!(ids(&discover(&[t.0.to_path_buf()])), ["x"]);
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
fn the_ci_image_name_is_read_from_the_app_folder() {
    let t = Tree::new("ci");
    let ci = "\
include:
  - project: common/cicd-tooling
    ref: latest
    file:
      - /templates/javascript/gitlab-ci-js.yml
.dev-apps:
  - ${IMAGE_NAME}:${CI_COMMIT_REF_NAME}
.base: &base
  script: [make]
build:
  <<: *base
  script:
    - !reference [.setup, script]
    - make build
variables:
  NODE_MANAGER: pnpm
  PORT: 3000
  DEPLOY: { value: x, description: y }
  IMAGE_NAME: nestjscanvasmodules
";
    t.file("nestjs-canvasmodules/package.json", r#"{"scripts": {"dev": "nest start"}}"#).file("nestjs-canvasmodules/.gitlab-ci.yml", ci);
    t.file("quoted/Makefile", "run:\n\ttrue\n").file("quoted/.gitlab-ci.yml", "variables:\n  IMAGE_NAME: \"boardservice\"\n");
    t.file("nested/Makefile", "run:\n\ttrue\n").file("nested/.gitlab-ci.yml", "job:\n  variables:\n    IMAGE_NAME: squish\n");
    t.file("number/Makefile", "run:\n\ttrue\n").file("number/.gitlab-ci.yml", "variables:\n  IMAGE_NAME: 12\n");
    t.file("broken/Makefile", "run:\n\ttrue\n").file("broken/.gitlab-ci.yml", "variables: [\n");
    t.file("variable/Makefile", "run:\n\ttrue\n").file("variable/.gitlab-ci.yml", "variables:\n  IMAGE_NAME: ${CI_PROJECT_NAME}-api\n");
    t.file("none/Makefile", "run:\n\ttrue\n");
    t.file("mono/package.json", "{}").file("mono/pnpm-workspace.yaml", "packages: ['packages/*']\n");
    t.file("mono/.gitlab-ci.yml", "variables:\n  IMAGE_NAME: monoimage\n");
    t.file("mono/packages/web/package.json", r#"{"scripts": {"dev": "vite"}}"#);
    let found = t.scan();
    assert!(found.problems.is_empty(), "a CI file is never a problem: {:?}", found.problems);
    assert_eq!(app(&found, "nestjs-canvasmodules").images, ["nestjscanvasmodules"], "tags, anchors and includes don't stop the read");
    assert_eq!(app(&found, "quoted").images, ["boardservice"]);
    assert_eq!(app(&found, "mono").images, ["monoimage"]);
    for id in ["nested", "number", "broken", "variable", "none", "mono/packages/web"] {
        assert!(app(&found, id).images.is_empty(), "{id}: no image");
    }
}
