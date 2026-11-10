//! The control API's git and port reads (R13.1): the workspace's apps' repositories only,
//! read as the git panel reads them (a real throwaway repository here), secret-named
//! paths counted, never listed; the workspace's ports with their holders.

use devdeck_api::ErrorKind;
use devdeck_api::types::{Branches, GitStatus, PortList, Repos};
use devdeck_app::api::Who;
use devdeck_app::api::dispatch::{self, Answer, CallCx};
use devdeck_app::shell::{Shell, WorkspaceSpec};
use devdeck_core::config::WorkspaceConfig;
use gpui::TestAppContext;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::time::{Duration, Instant};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "a")
        .env("GIT_AUTHOR_EMAIL", "a@x")
        .env("GIT_COMMITTER_NAME", "a")
        .env("GIT_COMMITTER_EMAIL", "a@x")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn agent() -> Who {
    Who::Terminal { workspace: "work".into(), n: 3, uid: "uid-3".into(), shell: 3, shell_start: 1, starts: 1 }
}

/// Calls `method`, polling (git runs on real threads) until it answers.
fn ask<T: serde::de::DeserializeOwned>(method: &str, params: Value, cx: &mut TestAppContext) -> Result<T, devdeck_api::ApiError> {
    let who = agent();
    let slot: Rc<RefCell<Option<Result<Value, devdeck_api::ApiError>>>> = Rc::default();
    match cx.update(|cx| dispatch::answer(&CallCx { who: &who, instance: "i", state: "demo", conn: None }, method, params, cx)) {
        Answer::Now(r) => *slot.borrow_mut() = Some(r),
        Answer::Later(task) => {
            let s = slot.clone();
            cx.update(|cx| cx.spawn(async move |_| *s.borrow_mut() = Some(task.await)).detach());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    while slot.borrow().is_none() {
        assert!(Instant::now() < deadline, "{method} didn't answer");
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
    let r = slot.borrow_mut().take().unwrap();
    r.map(|v| serde_json::from_value(v).unwrap())
}

#[gpui::test]
fn the_workspaces_repositories_read_with_secrets_hidden(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let base = cx.update(|cx| devdeck_app::test_dir("api-git", cx));
    let (origin, folder) = (base.join("origin.git"), base.join("projects"));
    std::fs::create_dir_all(&folder).unwrap();
    git(&base, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
    git(&folder, &["clone", "-q", origin.to_str().unwrap(), "web"]);
    let web = folder.join("web");
    std::fs::write(web.join("Makefile"), "dev:\n\techo dev\n").unwrap();
    git(&web, &["add", "."]);
    git(&web, &["commit", "-q", "-m", "one"]);
    git(&web, &["push", "-q", "-u", "origin", "main"]);
    git(&web, &["push", "-q", "origin", "main:feat"]);
    git(&web, &["fetch", "-q"]);
    std::fs::write(web.join(".env.local"), "TOKEN=x\n").unwrap();
    std::fs::write(web.join("notes.txt"), "hi\n").unwrap();
    let config = cx.update(|cx| devdeck_app::test_dir("api-git-config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let ws = WorkspaceConfig { id: "work".into(), name: "work".into(), folders: vec![folder.clone()], ..Default::default() };
    let shell = cx.update(|cx| Shell::start(vec![WorkspaceSpec::from_config(ws)], "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();

    let repos: Repos = ask("git.repos", json!({}), cx).unwrap();
    let root = web.canonicalize().unwrap().display().to_string();
    assert_eq!(repos.repos.iter().map(|r| r.root.as_str()).collect::<Vec<_>>(), [root.as_str()]);
    assert!(!repos.repos[0].apps.is_empty());

    let status: GitStatus = ask("git.status", json!({"root": root}), cx).unwrap();
    assert_eq!((status.head.kind.as_str(), status.head.name.as_str()), ("branch", "main"));
    assert_eq!(status.upstream.as_ref().map(|u| (u.name.as_str(), u.ahead, u.behind)), Some(("origin/main", 0, 0)));
    assert_eq!(status.changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["notes.txt"]);
    assert_eq!(status.hidden, 1, ".env.local is counted, not named");

    let local: Branches = ask("git.branches", json!({"root": root}), cx).unwrap();
    assert_eq!(local.branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["main"]);
    let all: Branches = ask("git.branches", json!({"root": root, "remote": true}), cx).unwrap();
    assert!(all.branches.iter().any(|b| b.remote && b.name.as_str() == "origin/feat"), "{:?}", all.branches);
    let one: Branches = ask("git.branches", json!({"root": root, "remote": true, "limit": 1}), cx).unwrap();
    assert!(one.truncated && one.branches.len() == 1);

    // A repository that isn't one of the workspace's apps': as if there were none.
    let other = ask::<GitStatus>("git.status", json!({"root": origin.display().to_string()}), cx).unwrap_err();
    assert_eq!(other.kind, ErrorKind::NotFound);
}

#[gpui::test]
fn the_demo_has_no_repositories_and_its_ports_answer_at_once(cx: &mut TestAppContext) {
    let config = cx.update(|cx| devdeck_app::test_dir("api-ports", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    let repos: Repos = ask("git.repos", json!({}), cx).unwrap();
    assert!(repos.repos.is_empty());
    let ports: PortList = ask("port.list", json!({}), cx).unwrap();
    assert!(!ports.ports.is_empty(), "the ports the demo's commands declare");
    assert!(ports.ports.windows(2).all(|w| w[0].port < w[1].port), "by port");
    let held = ports.ports.iter().find(|p| p.app.is_some()).expect("a running demo server holds one");
    assert!(held.holder.is_some());
    let one: devdeck_api::types::PortEntry = ask("port.get", json!({"port": held.port}), cx).unwrap();
    assert_eq!(one, *held);
    let foreign = ask::<devdeck_api::types::PortEntry>("port.get", json!({"port": 1}), cx).unwrap_err();
    assert_eq!(foreign.kind, ErrorKind::NotFound, "not one of the workspace's ports");
}
