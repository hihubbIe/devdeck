//! M3 step 3: the git panel (R6.10) over real temporary repositories (a bare "origin" and a
//! clone, made with `git`): one repository entity per root, read while looked at and when
//! git's files change; switching asks first, then switches; an operation under way blocks
//! every write but its abort. Git runs for real here, only in the tests' own folders.

use devdeck_app::repo::Repo;
use devdeck_app::shell::{Shell, WorkspaceSpec};
use devdeck_app::views::git_view::GitView;
use devdeck_core::config::WorkspaceConfig;
use devdeck_core::git::{Doing, Head, Op};
use devdeck_core::view_addr::ViewAddr;
use gpui::{Entity, TestAppContext};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// `git` in `dir`, isolated from the machine's config, failing the test when it fails.
fn git(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A workspace folder with a clone `web` (a Makefile makes it an app) of a bare origin,
/// with `main` and `feat` pushed.
fn repos(cx: &mut TestAppContext) -> (PathBuf, PathBuf) {
    let base = cx.update(|cx| devdeck_app::test_dir("git", cx));
    let origin = base.join("origin.git");
    let folder = base.join("projects");
    std::fs::create_dir_all(&folder).unwrap();
    git(&base, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
    git(&folder, &["clone", "-q", origin.to_str().unwrap(), "web"]);
    let web = folder.join("web");
    std::fs::write(web.join("Makefile"), "dev:\n\techo dev\n").unwrap();
    std::fs::write(web.join("a.txt"), "one\n").unwrap();
    git(&web, &["add", "."]);
    git(&web, &["commit", "-q", "-m", "one"]);
    git(&web, &["push", "-q", "-u", "origin", "main"]);
    git(&web, &["push", "-q", "origin", "main:feat"]);
    git(&web, &["fetch", "-q"]);
    (folder, web)
}

fn start(cx: &mut TestAppContext, folder: &Path) -> (Entity<Shell>, PathBuf) {
    cx.executor().allow_parking();
    let config = cx.update(|cx| devdeck_app::test_dir("config", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let ws = WorkspaceConfig { id: "work".into(), name: "work".into(), folders: vec![folder.to_path_buf()], ..Default::default() };
    let shell = cx.update(|cx| Shell::start(vec![WorkspaceSpec::from_config(ws)], "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    (shell, config)
}

/// Polls until `f` holds (git runs on real threads).
fn until(cx: &mut TestAppContext, what: &str, f: impl Fn(&TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f(cx) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
        cx.executor().advance_clock(Duration::from_millis(400));
        cx.run_until_parked();
    }
}

fn head(repo: &Entity<Repo>, cx: &TestAppContext) -> Option<Head> {
    cx.read(|cx| repo.read(cx).snapshot().map(|s| s.status.head.clone()))
}

fn open_panel(shell: &Entity<Shell>, web: &Path, cx: &mut TestAppContext) -> Entity<GitView> {
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = ViewAddr::Git { root: web.display().to_string() }.to_ref();
    let open = r.clone();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(open, l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    cx.read(|cx| deck.read(cx).view(&r).unwrap().downcast::<GitView>().unwrap())
}

#[gpui::test]
fn one_repository_per_root_shared_by_its_views_and_released_with_them(cx: &mut TestAppContext) {
    let (folder, web) = repos(cx);
    std::fs::create_dir_all(web.join("sub")).unwrap();
    let (shell, _) = start(cx, &folder);
    let s = cx.read(|cx| shell.read(cx).workspaces[0].services.clone());
    let a = cx.update(|cx| s.update(cx, |s, cx| s.repo(&web, cx))).expect("a repository");
    let b = cx.update(|cx| s.update(cx, |s, cx| s.repo(&web.join("sub"), cx))).expect("the same");
    assert_eq!(a.entity_id(), b.entity_id(), "one per root");
    assert!(cx.update(|cx| s.update(cx, |s, cx| s.repo(&folder, cx))).is_none(), "outside a repository");
    let id = a.entity_id();
    drop((a, b));
    cx.run_until_parked();
    let c = cx.update(|cx| s.update(cx, |s, cx| s.repo(&web, cx))).unwrap();
    assert_ne!(c.entity_id(), id, "released with its last holder");
}

#[gpui::test]
fn the_panel_reads_while_looked_at_and_switching_asks_first(cx: &mut TestAppContext) {
    let (folder, web) = repos(cx);
    let (shell, _) = start(cx, &folder);
    let panel = open_panel(&shell, &web, cx);
    let repo = cx.read(|cx| panel.read(cx).repo.clone());
    until(cx, "the repository read", |cx| head(&repo, cx).is_some());
    assert_eq!(head(&repo, cx), Some(Head::Branch("main".into())));
    let probe = Op::Push(devdeck_core::git::PushPlan::UpToDate { remote: String::new(), branch: String::new() });
    let why = cx.read(|cx| panel.read(cx).refusal(&probe, cx)).unwrap_or_default();
    assert!(why.contains("never pushes main"), "the push button says why before any fetch: {why}");
    let rows: Vec<String> = cx.read(|cx| panel.read(cx).rows(cx).iter().map(|r| r.branch.name.clone()).collect());
    assert_eq!(rows, ["main", "origin/feat"], "a remote branch without a local one; origin/main is main's");
    // Switching to the remote branch: asked in a modal, nothing done before it's confirmed.
    let main = cx.read(|cx| shell.read(cx).main_window().expect("main"));
    cx.update(|cx| {
        panel.update(cx, |p, cx| {
            p.selected = Some(1);
            p.switch_selected(cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| devdeck_ui::overlay::layer_state(main.into(), cx).1), 1, "a confirmation");
    assert_eq!(head(&repo, cx), Some(Head::Branch("main".into())), "nothing yet");
    cx.simulate_keystrokes(main.into(), "enter");
    until(cx, "on feat", |cx| head(&repo, cx) == Some(Head::Branch("feat".into())));
    assert_eq!(git(&web, &["rev-parse", "--abbrev-ref", "feat@{upstream}"]).trim(), "origin/feat", "tracking it");
}

#[gpui::test]
fn a_switch_made_in_a_terminal_shows_and_a_rebase_under_way_only_offers_its_abort(cx: &mut TestAppContext) {
    let (folder, web) = repos(cx);
    let (shell, config) = start(cx, &folder);
    let _watcher = cx.update(|cx| devdeck_app::watcher::start(&shell, config, cx)).expect("watching");
    let panel = open_panel(&shell, &web, cx);
    let repo = cx.read(|cx| panel.read(cx).repo.clone());
    until(cx, "read", |cx| head(&repo, cx).is_some());
    git(&web, &["switch", "-q", "feat"]);
    until(cx, "the terminal's switch seen", |cx| head(&repo, cx) == Some(Head::Branch("feat".into())));

    // A rebase stopped on a conflict.
    git(&web, &["switch", "-q", "main"]);
    std::fs::write(web.join("a.txt"), "main\n").unwrap();
    git(&web, &["commit", "-q", "-am", "main side"]);
    git(&web, &["switch", "-q", "feat"]);
    std::fs::write(web.join("a.txt"), "feat\n").unwrap();
    git(&web, &["commit", "-q", "-am", "feat side"]);
    let rebase = Command::new("git").arg("-C").arg(&web).args(["rebase", "main"]).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_COMMITTER_NAME", "a").env("GIT_COMMITTER_EMAIL", "a@x").output().unwrap();
    assert!(!rebase.status.success(), "stopped on the conflict");
    until(cx, "the rebase seen", |cx| cx.read(|cx| repo.read(cx).snapshot().is_some_and(|s| s.blocked.is_some())));
    cx.read(|cx| {
        let p = panel.read(cx);
        assert!(p.refusal(&Op::Fetch, cx).is_some() && p.refusal(&Op::Pull("feat".into()), cx).is_some() && p.refusal(&Op::Switch("main".into()), cx).is_some());
        assert_eq!(p.refusal(&Op::Abort(Doing::Rebasing), cx), Some(devdeck_app::edit_mode::OFF.into()), "an abort resets changes: edit mode only");
    });
    cx.update(|cx| devdeck_app::edit_mode::set(true, cx));
    cx.read(|cx| assert_eq!(panel.read(cx).refusal(&Op::Abort(Doing::Rebasing), cx), None, "only the abort"));
    cx.update(|cx| repo.update(cx, |r, cx| r.run(Op::Abort(Doing::Rebasing), "Abort".into(), "aborting…".into(), cx))).unwrap();
    until(cx, "aborted", |cx| cx.read(|cx| repo.read(cx).snapshot().is_some_and(|s| s.blocked.is_none())));
    assert_eq!(std::fs::read_to_string(web.join("a.txt")).unwrap(), "feat\n", "back where it was");
}

#[gpui::test]
fn a_new_branch_is_published_after_asking_and_a_remote_ahead_is_refused(cx: &mut TestAppContext) {
    let (folder, web) = repos(cx);
    let (shell, _) = start(cx, &folder);
    let main = cx.read(|cx| shell.read(cx).main_window().expect("main"));
    let panel = open_panel(&shell, &web, cx);
    let repo = cx.read(|cx| panel.read(cx).repo.clone());
    until(cx, "read", |cx| head(&repo, cx).is_some());
    let layers = |cx: &mut TestAppContext| cx.update(|cx| devdeck_ui::overlay::layer_state(main.into(), cx).1);

    // New branch: a modal asks its name; Enter creates it and switches to it.
    cx.update(|cx| panel.update(cx, |p, cx| p.new_branch(cx)));
    cx.run_until_parked();
    assert_eq!(layers(cx), 1);
    cx.simulate_input(main.into(), "topic");
    cx.simulate_keystrokes(main.into(), "enter");
    until(cx, "on topic", |cx| head(&repo, cx) == Some(Head::Branch("topic".into())));
    assert_eq!(layers(cx), 0);

    // Push: the remote is checked, then a confirmation to publish; nothing before it.
    cx.update(|cx| panel.update(cx, |p, cx| p.push(cx)));
    let deadline = Instant::now() + Duration::from_secs(10);
    while layers(cx) != 1 {
        assert!(Instant::now() < deadline, "timed out waiting for the publish confirmation");
        std::thread::sleep(Duration::from_millis(20));
        cx.run_until_parked();
    }
    assert!(git(&web, &["ls-remote", "--heads", "origin", "topic"]).trim().is_empty(), "nothing pushed yet");
    cx.simulate_keystrokes(main.into(), "enter");
    until(cx, "published", |_| !git(&web, &["ls-remote", "--heads", "origin", "topic"]).trim().is_empty());
    until(cx, "tracking it", |cx| cx.read(|cx| repo.read(cx).snapshot().is_some_and(|s| s.status.upstream.as_ref().is_some_and(|u| u.name == "origin/topic"))));

    // Someone else pushes to topic: our push is refused without a modal, and the remote keeps theirs.
    let other = folder.join("other");
    git(&folder, &["clone", "-q", "-b", "topic", folder.parent().unwrap().join("origin.git").to_str().unwrap(), "other"]);
    std::fs::write(other.join("b.txt"), "theirs\n").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-q", "-m", "theirs"]);
    git(&other, &["push", "-q", "origin", "topic"]);
    let theirs = git(&other, &["rev-parse", "HEAD"]);
    cx.update(|cx| panel.update(cx, |p, cx| p.push(cx)));
    until(cx, "the check done", |cx| cx.read(|cx| repo.read(cx).busy.is_none()));
    cx.run_until_parked();
    assert_eq!(layers(cx), 0, "refused: no confirmation offered");
    assert_eq!(git(&web, &["ls-remote", "origin", "refs/heads/topic"]).split_whitespace().next(), theirs.split_whitespace().next(), "the remote keeps their commit");
}

#[gpui::test]
fn a_second_operation_waits_for_the_first(cx: &mut TestAppContext) {
    let (folder, web) = repos(cx);
    let (shell, _) = start(cx, &folder);
    let panel = open_panel(&shell, &web, cx);
    let repo = cx.read(|cx| panel.read(cx).repo.clone());
    until(cx, "read", |cx| cx.read(|cx| repo.read(cx).full().is_some()));
    cx.update(|cx| repo.update(cx, |r, cx| r.run(Op::Fetch, "Fetch".into(), "fetching…".into(), cx))).unwrap();
    let second = cx.update(|cx| repo.update(cx, |r, cx| r.run(Op::Switch("feat".into()), "Switch".into(), "switching…".into(), cx)));
    assert_eq!(second, Err("fetching… first".into()), "one at a time");
    until(cx, "the fetch done", |cx| cx.read(|cx| repo.read(cx).busy.is_none()));
    assert_eq!(head(&repo, cx), Some(Head::Branch("main".into())), "the refused switch didn't happen");
}
