//! `devdeck_core::git` against real repositories made in temporary folders: a bare
//! `origin` and clones of it. A test binary of its own: these spawn many processes, and a
//! process forked while another test holds a socket would hold it too (`ports` tests).

use devdeck_core::git::*;
use devdeck_core::testing::TempDir;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// A bare `origin` with one commit on `main` and a `feat` branch, and `work`, a clone
/// of it. Every git call (the fixture's and devdeck's) gets the same isolated
/// variables: no user or system config, a fixed identity and dates, C messages.
struct Repos {
    dir: TempDir,
    vars: Vec<(String, String)>,
}

impl Repos {
    fn new(tag: &str) -> Repos {
        let dir = TempDir::new(&format!("git-{tag}"));
        let mut vars: Vec<(String, String)> = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "Ada"),
            ("GIT_AUTHOR_EMAIL", "ada@example.com"),
            ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z"),
            ("GIT_COMMITTER_NAME", "Ada"),
            ("GIT_COMMITTER_EMAIL", "ada@example.com"),
            ("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z"),
            ("LC_ALL", "C"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        vars.push(("PATH".into(), std::env::var("PATH").unwrap_or_default()));
        vars.push(("HOME".into(), dir.display().to_string()));
        let r = Repos { dir, vars };
        r.sh(".", &["init", "-q", "--bare", "-b", "main", "origin"]);
        r.sh(".", &["clone", "-q", "origin", "work"]);
        r.commit("work", "a.txt", "one\n", "first");
        r.sh("work", &["push", "-q", "-u", "origin", "main"]);
        r.sh("work", &["push", "-q", "origin", "main:feat"]);
        r.sh("work", &["fetch", "-q"]);
        r
    }

    /// `work` on a branch `dev`, published and tracked (devdeck never pushes main).
    fn on_dev(&self) {
        self.sh("work", &["switch", "-q", "-c", "dev"]);
        self.sh("work", &["push", "-q", "-u", "origin", "dev"]);
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    fn run(&self, repo: &str, args: &[&str]) -> std::process::Output {
        Command::new("git").arg("-C").arg(self.path(repo)).args(args).env_clear().envs(self.vars.iter().map(|(k, v)| (k, v))).output().unwrap()
    }

    /// Commits as Bob (a colleague).
    fn commit_as_bob(&self, repo: &str, file: &str, text: &str, message: &str) {
        self.write(&format!("{repo}/{file}"), text);
        self.sh(repo, &["add", "--", file]);
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(self.path(repo)).args(["commit", "-q", "-m", message]).env_clear().envs(self.vars.iter().map(|(k, v)| (k, v)));
        let out = cmd.env("GIT_AUTHOR_NAME", "Bob").env("GIT_AUTHOR_EMAIL", "bob@example.com").output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    /// A hook that runs `body` (`sh`).
    fn hook(&self, repo: &str, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.path(&format!("{repo}/.git/hooks/{name}"));
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// git's output; panics when it fails.
    fn sh(&self, repo: &str, args: &[&str]) -> String {
        let out = self.run(repo, args);
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn write(&self, rel: &str, text: &str) {
        std::fs::write(self.path(rel), text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).unwrap()
    }

    fn commit(&self, repo: &str, file: &str, text: &str, message: &str) {
        self.write(&format!("{repo}/{file}"), text);
        self.sh(repo, &["add", "--", file]);
        self.sh(repo, &["commit", "-q", "-m", message]);
    }

    fn clone(&self, name: &str) {
        self.sh(".", &["clone", "-q", "origin", name]);
    }

    fn git(&self, repo: &str) -> Git {
        Git::open(&self.path(repo), None, self.vars.clone()).unwrap().0
    }

    fn rev(&self, repo: &str, rev: &str) -> String {
        self.sh(repo, &["rev-parse", rev])
    }
}

fn change(path: &str, kind: ChangeKind, staged: bool) -> Change {
    Change { path: path.into(), from: None, kind, staged }
}

#[test]
fn open_finds_the_root_and_git_dirs() {
    let r = Repos::new("open");
    std::fs::create_dir(r.path("work/sub")).unwrap();
    let (git, dirs) = Git::open(&r.path("work/sub"), None, r.vars.clone()).unwrap();
    assert_eq!(git.root(), r.path("work"));
    assert_eq!(dirs, GitDirs { root: r.path("work"), git_dir: r.path("work/.git"), common_dir: r.path("work/.git") });

    r.sh("work", &["worktree", "add", "-q", "-b", "wt", "../wt"]);
    let (_, dirs) = Git::open(&r.path("wt"), None, r.vars.clone()).unwrap();
    assert_eq!(dirs, GitDirs { root: r.path("wt"), git_dir: r.path("work/.git/worktrees/wt"), common_dir: r.path("work/.git") });

    std::fs::create_dir(r.path("plain")).unwrap();
    let e = Git::open(&r.path("plain"), None, r.vars.clone()).unwrap_err();
    assert!(e.contains("not a git repository"), "{e}");
}

#[test]
fn a_repository_is_looked_for_up_to_the_ceiling_only() {
    let r = Repos::new("ceiling");
    std::fs::create_dir_all(r.path("work/ws/app/src")).unwrap();
    let opened = Git::open(&r.path("work/ws/app/src"), Some(&r.path("work/ws")), r.vars.clone());
    assert!(opened.unwrap_err().contains("not a git repository"), "the repository above the ceiling isn't found");
    let (git, _) = Git::open(&r.path("work/ws/app/src"), Some(&r.path("work")), r.vars.clone()).unwrap();
    assert_eq!(git.root(), r.path("work"), "a repository at the ceiling folder itself opens");
    assert!(Git::open(&r.path("work/ws/app/src"), None, r.vars.clone()).is_ok());
}

#[test]
fn a_clean_clone_tracks_its_upstream() {
    let r = Repos::new("clean");
    let s = r.git("work").status().unwrap();
    assert_eq!(
        s,
        Status { head: Head::Branch("main".into()), upstream: Some(Upstream { name: "origin/main".into(), ahead: 0, behind: 0, gone: false }), changes: vec![], doing: None }
    );
    assert_eq!(r.git("work").head(), Ok(Head::Branch("main".into())));
}

#[test]
fn changes_of_each_kind() {
    let r = Repos::new("changes");
    r.commit("work", "old name.txt", "rename me\n", "more");
    r.commit("work", "gone.txt", "x\n", "more");
    r.commit("work", "staged.txt", "x\n", "more");
    r.write("work/a.txt", "two\n");
    r.write("work/staged.txt", "y\n");
    r.sh("work", &["add", "staged.txt"]);
    r.write("work/new.txt", "n\n");
    r.sh("work", &["add", "new.txt"]);
    std::fs::remove_file(r.path("work/gone.txt")).unwrap();
    r.sh("work", &["mv", "old name.txt", "new name.txt"]);
    r.write("work/untracked file.txt", "u\n");
    let mut changes = r.git("work").status().unwrap().changes;
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    let renamed = Change { path: "new name.txt".into(), from: Some("old name.txt".into()), kind: ChangeKind::Renamed, staged: true };
    assert_eq!(
        changes,
        [
            change("a.txt", ChangeKind::Modified, false),
            change("gone.txt", ChangeKind::Deleted, false),
            renamed,
            change("new.txt", ChangeKind::Added, true),
            change("staged.txt", ChangeKind::Modified, true),
            change("untracked file.txt", ChangeKind::Untracked, false),
        ]
    );
}

#[test]
fn a_conflicted_merge_is_shown_and_aborted() {
    let r = Repos::new("merge");
    r.commit("work", "a.txt", "theirs\n", "theirs");
    r.sh("work", &["switch", "-q", "-c", "side", "HEAD~1"]);
    r.commit("work", "a.txt", "ours\n", "ours");
    let start = r.rev("work", "HEAD");
    assert!(!r.run("work", &["merge", "-q", "main"]).status.success());
    let git = r.git("work");
    let s = git.status().unwrap();
    assert_eq!(s.changes, [change("a.txt", ChangeKind::Conflicted, false)]);
    assert_eq!(s.doing, Some(Doing::Merging));
    assert_eq!(git.writes_blocked(), Ok(Some("a merge is under way: finish it, or abort".into())));
    assert_eq!(git.run(&Op::Abort(Doing::Rebasing)), Err("no rebase under way".into()));
    git.run(&Op::Abort(Doing::Merging)).unwrap();
    assert_eq!(r.rev("work", "HEAD"), start);
    assert!(!r.path("work/.git/MERGE_HEAD").exists());
    assert_eq!(r.read("work/a.txt"), "ours\n");
    assert_eq!(git.status().unwrap().doing, None);
}

#[test]
fn a_conflicted_rebase_blocks_every_write_until_aborted() {
    let r = Repos::new("rebase");
    r.commit("work", "a.txt", "theirs\n", "theirs");
    r.sh("work", &["switch", "-q", "-c", "side", "HEAD~1"]);
    r.commit("work", "a.txt", "ours\n", "ours");
    let start = r.rev("work", "HEAD");
    assert!(!r.run("work", &["rebase", "main"]).status.success());
    let git = r.git("work");
    assert_eq!(git.status().unwrap().doing, Some(Doing::Rebasing));
    let why = "a rebase is under way: finish it, or abort".to_string();
    let ops = [Op::Fetch, Op::Pull("side".into()), Op::Switch("main".into()), Op::Track("origin/feat".into()), Op::Create("x".into()), Op::Push(PushPlan::Publish { remote: "origin".into(), branch: "side".into() })];
    for op in &ops {
        assert_eq!(git.run(op), Err(why.clone()), "{op:?}");
    }
    assert_eq!(git.push_plan(), Err(why));
    assert!(!r.path("work/.git/refs/heads/x").exists());
    git.run(&Op::Abort(Doing::Rebasing)).unwrap();
    assert_eq!(r.rev("work", "HEAD"), start);
    assert_eq!(r.sh("work", &["symbolic-ref", "--short", "HEAD"]), "side");
    assert!(!r.path("work/.git/rebase-merge").exists() && !r.path("work/.git/rebase-apply").exists());
    assert_eq!(git.writes_blocked(), Ok(None));
}

#[test]
fn detached_and_unborn_heads() {
    let r = Repos::new("heads");
    r.sh("work", &["switch", "-q", "--detach", "HEAD"]);
    let short = r.sh("work", &["rev-parse", "--short", "HEAD"]);
    assert_eq!(r.git("work").status().unwrap().head, Head::Detached(short.clone()));
    assert_eq!(r.git("work").status().unwrap().upstream, None);
    assert_eq!(r.git("work").head(), Ok(Head::Detached(short)));

    r.sh(".", &["init", "-q", "-b", "trunk", "fresh"]);
    let s = r.git("fresh").status().unwrap();
    assert_eq!((s.head, s.upstream), (Head::Unborn("trunk".into()), None));
    assert_eq!(r.git("fresh").head(), Ok(Head::Unborn("trunk".into())));
    assert_eq!(r.git("fresh").branches(), Ok(vec![]));
}

#[test]
fn ahead_behind_gone_and_no_upstream() {
    let r = Repos::new("ab");
    r.clone("other");
    r.commit("other", "b.txt", "b\n", "theirs");
    r.sh("other", &["push", "-q", "origin", "main"]);
    r.sh("other", &["push", "-q", "origin", "--delete", "feat"]);
    r.commit("work", "c.txt", "c\n", "mine");
    r.sh("work", &["branch", "-q", "--track", "feat", "origin/feat"]);
    r.git("work").run(&Op::Fetch).unwrap();
    let up = r.git("work").status().unwrap().upstream;
    assert_eq!(up, Some(Upstream { name: "origin/main".into(), ahead: 1, behind: 1, gone: false }));

    r.sh("work", &["switch", "-q", "feat"]);
    let up = r.git("work").status().unwrap().upstream;
    assert_eq!(up, Some(Upstream { name: "origin/feat".into(), ahead: 0, behind: 0, gone: true }));

    r.sh("work", &["switch", "-q", "-c", "local"]);
    assert_eq!(r.git("work").status().unwrap().upstream, None);
    assert_eq!(r.git("work").run(&Op::Pull("local".into())), Err("no upstream to pull from".into()));
    assert_eq!(r.git("work").run(&Op::Pull("main".into())), Err("main isn't the current branch any more".into()));
}

#[test]
fn branches_local_then_remote() {
    let r = Repos::new("branches");
    r.sh("work", &["remote", "set-head", "origin", "main"]);
    r.sh("work", &["branch", "-q", "--track", "feat", "origin/feat"]);
    r.sh("work", &["worktree", "add", "-q", "-b", "elsewhere", "../wt"]);
    r.commit("wt", "w.txt", "w\n", "in the worktree");
    let branches = r.git("work").branches().unwrap();
    let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["elsewhere", "feat", "main", "origin/feat", "origin/main"], "local first; the remote HEAD left out");
    let main = &branches[2];
    assert_eq!(
        main,
        &Branch {
            name: "main".into(),
            remote: None,
            upstream: Some("origin/main".into()),
            commit: r.sh("work", &["rev-parse", "--short", "main"]),
            subject: "first".into(),
            author: "Ada".into(),
            at: 1767225600,
            current: true,
            worktree: None,
        }
    );
    assert_eq!(branches[0].worktree, Some(r.path("wt")));
    assert_eq!((branches[0].current, branches[0].subject.as_str()), (false, "in the worktree"));
    assert_eq!(branches[1].upstream.as_deref(), Some("origin/feat"));
    assert_eq!((branches[3].remote.as_deref(), branches[3].upstream.as_deref()), (Some("origin"), None));
}

#[test]
fn switch_track_and_create() {
    let r = Repos::new("switch");
    r.sh("work", &["branch", "-q", "local"]);
    let git = r.git("work");
    git.run(&Op::Switch("local".into())).unwrap();
    assert_eq!(git.head(), Ok(Head::Branch("local".into())));
    assert_eq!(git.run(&Op::Switch("-x".into())), Err("-x: a name can't start with -".into()));
    assert!(git.run(&Op::Switch("feat".into())).is_err(), "a remote branch isn't switched to without Track");

    git.run(&Op::Track("origin/feat".into())).unwrap();
    assert_eq!(git.head(), Ok(Head::Branch("feat".into())));
    assert_eq!(git.status().unwrap().upstream.map(|u| u.name), Some("origin/feat".into()));
    assert_eq!(git.run(&Op::Track("main".into())), Err("main isn't a remote branch".into()));

    git.run(&Op::Create("feature/new".into())).unwrap();
    assert_eq!(git.head(), Ok(Head::Branch("feature/new".into())));
    for bad in ["@", "@{-1}", "-x", "a..b", "HEAD", "", "a b"] {
        assert!(git.run(&Op::Create(bad.into())).is_err(), "{bad:?} refused");
        assert!(git.check_branch_name(bad).is_err(), "{bad:?} refused");
    }
    assert_eq!(git.run(&Op::Create("main".into())), Err("branch main exists".into()));
    assert_eq!(git.head(), Ok(Head::Branch("feature/new".into())));
    assert_eq!(git.check_branch_name("fine-name"), Ok(()));
}

#[test]
fn a_switch_over_conflicting_changes_is_refused() {
    let r = Repos::new("dirty");
    r.sh("work", &["switch", "-q", "-c", "other"]);
    r.commit("work", "a.txt", "other\n", "other");
    r.sh("work", &["switch", "-q", "main"]);
    r.write("work/a.txt", "local edit\n");
    let git = r.git("work");
    let e = git.run(&Op::Switch("other".into())).unwrap_err();
    assert!(e.contains("would be overwritten"), "{e}");
    assert_eq!(git.head(), Ok(Head::Branch("main".into())));
    assert_eq!(r.read("work/a.txt"), "local edit\n");
}

#[test]
fn an_ignored_file_another_branch_tracks_is_never_overwritten() {
    let r = Repos::new("ignored");
    r.sh("work", &["switch", "-q", "-c", "withenv"]);
    r.commit("work", ".env", "committed\n", "track .env");
    r.sh("work", &["switch", "-q", "main"]);
    r.commit("work", ".gitignore", ".env\n", "ignore .env");
    r.write("work/.env", "secret\n");
    let git = r.git("work");
    assert!(git.status().unwrap().changes.is_empty(), ".env is ignored");
    let e = git.run(&Op::Switch("withenv".into())).unwrap_err();
    assert!(e.contains(".env"), "{e}");
    assert_eq!(r.read("work/.env"), "secret\n");
    assert_eq!(git.head(), Ok(Head::Branch("main".into())));
}

#[test]
fn pull_fast_forwards_only() {
    let r = Repos::new("pull");
    r.clone("other");
    r.commit("other", "b.txt", "b\n", "theirs");
    r.sh("other", &["push", "-q", "origin", "main"]);
    let git = r.git("work");
    git.run(&Op::Pull("main".into())).unwrap();
    assert_eq!(r.rev("work", "HEAD"), r.rev("other", "HEAD"));

    r.commit("other", "c.txt", "c\n", "theirs again");
    r.sh("other", &["push", "-q", "origin", "main"]);
    r.commit("work", "d.txt", "d\n", "mine");
    let mine = r.rev("work", "HEAD");
    let e = git.run(&Op::Pull("main".into())).unwrap_err();
    assert!(e.contains("Not possible to fast-forward"), "{e}");
    assert_eq!(r.rev("work", "HEAD"), mine);
    assert!(git.status().unwrap().changes.is_empty());
}

#[test]
fn pull_ignores_rebase_and_autostash_config() {
    let r = Repos::new("pull-config");
    for (key, value) in [("pull.rebase", "true"), ("rebase.autoStash", "true"), ("merge.autoStash", "true")] {
        r.sh("work", &["config", key, value]);
    }
    r.clone("other");
    r.commit("other", "b.txt", "b\n", "theirs");
    r.sh("other", &["push", "-q", "origin", "main"]);
    r.write("work/a.txt", "local edit\n");
    let git = r.git("work");
    git.run(&Op::Pull("main".into())).unwrap();
    assert_eq!(r.rev("work", "HEAD"), r.rev("other", "HEAD"));
    assert_eq!(r.read("work/a.txt"), "local edit\n", "carried along by the fast-forward");
    assert!(!r.path("work/.git/refs/stash").exists(), "no stash");

    // Diverged: refused, not rebased.
    r.commit("other", "c.txt", "c\n", "theirs again");
    r.sh("other", &["push", "-q", "origin", "main"]);
    r.commit("work", "d.txt", "d\n", "mine");
    let mine = r.rev("work", "HEAD");
    assert!(git.run(&Op::Pull("main".into())).is_err());
    assert_eq!(r.rev("work", "HEAD"), mine);
    assert_eq!(git.status().unwrap().doing, None);
    assert!(!r.path("work/.git/refs/stash").exists());
}

#[test]
fn fetch_adds_and_prunes_remote_branches() {
    let r = Repos::new("fetch");
    r.clone("other");
    r.sh("other", &["push", "-q", "origin", "main:fresh"]);
    r.sh("other", &["push", "-q", "origin", "--delete", "feat"]);
    let git = r.git("work");
    git.run(&Op::Fetch).unwrap();
    assert!(git.has_ref("origin/fresh"));
    assert!(!git.has_ref("origin/feat"));
}

#[test]
fn refs_of_each_kind() {
    let r = Repos::new("refs");
    r.sh("work", &["tag", "v1"]);
    let git = r.git("work");
    assert!(git.has_ref("main") && git.has_ref("origin/feat") && git.has_ref("v1"));
    assert!(!git.has_ref("nope") && !git.has_ref("feat") && !git.has_ref("-q"));
}

fn plan_of(r: &Repos, repo: &str) -> PushPlan {
    r.git(repo).push_plan().unwrap()
}

#[test]
fn push_publishes_sends_and_says_up_to_date() {
    let r = Repos::new("push");
    let git = r.git("work");
    r.sh("work", &["switch", "-q", "-c", "topic"]);
    r.commit("work", "t.txt", "t\n", "topic");
    let plan = git.push_plan().unwrap();
    assert_eq!(plan, PushPlan::Publish { remote: "origin".into(), branch: "topic".into() });
    git.run(&Op::Push(plan)).unwrap();
    assert_eq!(r.rev("origin", "topic"), r.rev("work", "HEAD"));
    assert_eq!(git.status().unwrap().upstream.map(|u| u.name), Some("origin/topic".into()), "tracked from then on");

    r.commit("work", "t.txt", "t2\n", "topic 2");
    r.commit("work", "t.txt", "t3\n", "topic 3");
    let plan = git.push_plan().unwrap();
    assert_eq!(plan, PushPlan::Send { remote: "origin".into(), branch: "topic".into(), head: r.rev("work", "HEAD"), commits: 2 });
    git.run(&Op::Push(plan)).unwrap();
    assert_eq!(r.rev("origin", "topic"), r.rev("work", "HEAD"));

    let plan = git.push_plan().unwrap();
    assert_eq!(plan, PushPlan::UpToDate { remote: "origin".into(), branch: "topic".into() });
    assert!(git.run(&Op::Push(plan)).is_err());
}

#[test]
fn main_and_master_are_never_pushed() {
    let r = Repos::new("push-main");
    let git = r.git("work");
    r.commit("work", "m.txt", "m\n", "on main");
    let before = r.rev("origin", "main");
    assert!(git.push_plan().unwrap_err().contains("never pushes main"), "no plan, no fetch");
    // A plan made elsewhere for main is refused too, whatever its kind.
    let head = r.rev("work", "HEAD");
    for plan in [
        PushPlan::Publish { remote: "origin".into(), branch: "main".into() },
        PushPlan::Send { remote: "origin".into(), branch: "main".into(), head: head.clone(), commits: 1 },
        PushPlan::Replace { remote: "origin".into(), branch: "main".into(), expected: before.clone(), head: head.clone(), replaced: vec![] },
    ] {
        assert!(git.run(&Op::Push(plan)).unwrap_err().contains("never pushes main"));
    }
    assert_eq!(r.rev("origin", "main"), before, "nothing pushed");
    r.sh("work", &["switch", "-q", "-c", "Master"]);
    assert!(git.push_plan().unwrap_err().contains("never pushes Master"), "any case");
}

#[test]
fn push_refuses_when_the_remote_is_ahead() {
    let r = Repos::new("push-behind");
    r.on_dev();
    r.clone("other");
    r.sh("other", &["switch", "-q", "dev"]);
    r.commit("other", "b.txt", "b\n", "theirs");
    r.sh("other", &["push", "-q", "origin", "dev"]);
    let remote = r.rev("origin", "dev");
    let plan = plan_of(&r, "work");
    assert_eq!(plan, PushPlan::Behind { remote: "origin".into(), branch: "dev".into(), commits: 1 });
    assert!(r.git("work").run(&Op::Push(plan)).is_err());
    assert_eq!(r.rev("origin", "dev"), remote);

    // Diverged with commits never had locally: still refused.
    r.commit("work", "c.txt", "c\n", "mine");
    let plan = plan_of(&r, "work");
    assert_eq!(plan, PushPlan::Behind { remote: "origin".into(), branch: "dev".into(), commits: 1 });
    assert!(r.git("work").run(&Op::Push(plan)).is_err());
    assert_eq!(r.rev("origin", "dev"), remote);
}

#[test]
fn push_replaces_after_an_amend() {
    let r = Repos::new("push-amend");
    r.on_dev();
    r.commit("work", "b.txt", "b\n", "draft");
    r.sh("work", &["push", "-q"]);
    let old = r.rev("work", "HEAD");
    r.write("work/b.txt", "better\n");
    r.sh("work", &["commit", "-q", "-a", "--amend", "-m", "final"]);
    let plan = plan_of(&r, "work");
    let replaced = vec![Commit { id: old.clone(), subject: "draft".into(), author: "Ada".into() }];
    assert_eq!(plan, PushPlan::Replace { remote: "origin".into(), branch: "dev".into(), expected: old.clone(), head: r.rev("work", "HEAD"), replaced });
    r.git("work").run(&Op::Push(plan)).unwrap();
    assert_eq!(r.rev("origin", "dev"), r.rev("work", "HEAD"));
    let backup = r.sh("work", &["for-each-ref", "--format=%(objectname)", "refs/devdeck/replaced/dev/"]);
    assert_eq!(backup, old, "a backup ref keeps the replaced tip");
}

#[test]
fn push_replaces_after_a_rebase() {
    let r = Repos::new("push-rebase");
    r.sh("work", &["switch", "-q", "-c", "topic"]);
    r.commit("work", "t.txt", "t\n", "topic");
    r.sh("work", &["push", "-q", "-u", "origin", "topic"]);
    let old = r.rev("work", "HEAD");
    r.sh("work", &["switch", "-q", "main"]);
    r.commit("work", "m.txt", "m\n", "main moves");
    r.sh("work", &["switch", "-q", "topic"]);
    r.sh("work", &["rebase", "-q", "main"]);
    let plan = plan_of(&r, "work");
    let replaced = vec![Commit { id: old.clone(), subject: "topic".into(), author: "Ada".into() }];
    assert_eq!(plan, PushPlan::Replace { remote: "origin".into(), branch: "topic".into(), expected: old.clone(), head: r.rev("work", "HEAD"), replaced });
    r.git("work").run(&Op::Push(plan)).unwrap();
    assert_eq!(r.rev("origin", "topic"), r.rev("work", "HEAD"));
    assert_eq!(r.sh("work", &["log", "-1", "--format=%s", "HEAD"]), "topic", "the remote's commit, rebased, is in the new tip");
}

#[test]
fn a_replace_is_refused_when_someone_pushed_meanwhile() {
    let r = Repos::new("push-lease");
    r.on_dev();
    r.commit("work", "b.txt", "b\n", "draft");
    r.sh("work", &["push", "-q"]);
    r.sh("work", &["commit", "-q", "--amend", "-m", "amended"]);
    let plan = plan_of(&r, "work");
    assert!(matches!(plan, PushPlan::Replace { .. }), "{plan:?}");
    r.clone("other");
    r.sh("other", &["switch", "-q", "dev"]);
    r.commit("other", "o.txt", "o\n", "pushed meanwhile");
    r.sh("other", &["push", "-q", "origin", "dev"]);
    let theirs = r.rev("other", "HEAD");
    let e = r.git("work").run(&Op::Push(plan)).unwrap_err();
    assert!(e.contains("rejected"), "{e}");
    assert_eq!(r.rev("origin", "dev"), theirs);
}

#[test]
fn the_push_remote_is_chosen_or_refused() {
    let r = Repos::new("push-remote");
    r.sh("work", &["switch", "-q", "-c", "topic"]);
    r.sh(".", &["init", "-q", "--bare", "-b", "main", "mirror"]);
    r.sh("work", &["remote", "add", "mirror", "../mirror"]);
    let e = r.git("work").push_plan().unwrap_err();
    assert_eq!(e, "several remotes (mirror, origin) and none chosen for topic: set its upstream or remote.pushDefault");
    r.sh("work", &["config", "remote.pushDefault", "mirror"]);
    assert_eq!(plan_of(&r, "work"), PushPlan::Publish { remote: "mirror".into(), branch: "topic".into() });
    r.sh("work", &["config", "branch.topic.pushRemote", "origin"]);
    assert_eq!(plan_of(&r, "work"), PushPlan::Publish { remote: "origin".into(), branch: "topic".into() });
    r.sh("work", &["switch", "-q", "feat"]);
    assert_eq!(plan_of(&r, "work"), PushPlan::UpToDate { remote: "origin".into(), branch: "feat".into() }, "the upstream's remote first");
    r.sh("work", &["switch", "-q", "--detach"]);
    assert!(r.git("work").push_plan().unwrap_err().starts_with("detached at"));
}

#[test]
fn a_push_plan_for_another_branch_is_refused() {
    let r = Repos::new("push-stale");
    r.on_dev();
    let plan = PushPlan::Publish { remote: "origin".into(), branch: "elsewhere".into() };
    assert_eq!(r.git("work").run(&Op::Push(plan)), Err("elsewhere isn't the current branch any more".into()));
    let head = r.rev("work", "HEAD");
    let plan = PushPlan::Replace { remote: "origin".into(), branch: "dev".into(), expected: String::new(), head, replaced: vec![] };
    assert!(r.git("work").run(&Op::Push(plan)).unwrap_err().contains("full commit id"));
}

#[test]
fn a_colleague_s_commit_dropped_locally_is_never_replaced() {
    let r = Repos::new("push-dropped");
    r.on_dev();
    r.clone("other");
    r.sh("other", &["switch", "-q", "dev"]);
    r.commit_as_bob("other", "x.txt", "x\n", "bob's work");
    r.sh("other", &["push", "-q", "origin", "dev"]);
    let theirs = r.rev("origin", "dev");
    let git = r.git("work");
    git.run(&Op::Pull("dev".into())).unwrap();
    r.sh("work", &["reset", "-q", "--hard", "HEAD~1"]);
    r.commit("work", "c.txt", "c\n", "mine");
    let plan = git.push_plan().unwrap();
    assert_eq!(plan, PushPlan::Behind { remote: "origin".into(), branch: "dev".into(), commits: 1 }, "in the reflog, but Bob's and not in the branch");
    assert!(git.run(&Op::Push(plan)).is_err());
    assert_eq!(r.rev("origin", "dev"), theirs);
}

#[test]
fn a_colleague_s_commit_rebased_is_replaced() {
    let r = Repos::new("push-rebased-bob");
    r.clone("other");
    r.sh("other", &["switch", "-q", "-c", "topic"]);
    r.commit_as_bob("other", "x.txt", "x\n", "bob's work");
    r.sh("other", &["push", "-q", "origin", "topic"]);
    let bobs = r.rev("other", "HEAD");
    let git = r.git("work");
    git.run(&Op::Fetch).unwrap();
    git.run(&Op::Track("origin/topic".into())).unwrap();
    r.sh("work", &["switch", "-q", "main"]);
    r.commit("work", "m.txt", "m\n", "main moves");
    r.sh("work", &["switch", "-q", "topic"]);
    r.sh("work", &["rebase", "-q", "main"]);
    let plan = git.push_plan().unwrap();
    let replaced = vec![Commit { id: bobs.clone(), subject: "bob's work".into(), author: "Bob".into() }];
    assert_eq!(plan, PushPlan::Replace { remote: "origin".into(), branch: "topic".into(), expected: bobs, head: r.rev("work", "HEAD"), replaced }, "the same patch is in the branch");
    git.run(&Op::Push(plan)).unwrap();
    assert_eq!(r.rev("origin", "topic"), r.rev("work", "HEAD"));
}

#[test]
fn a_plan_made_before_head_moved_is_refused() {
    let r = Repos::new("push-moved");
    r.on_dev();
    let git = r.git("work");
    r.commit("work", "b.txt", "b\n", "one more");
    let plan = git.push_plan().unwrap();
    assert!(matches!(plan, PushPlan::Send { .. }), "{plan:?}");
    let remote = r.rev("origin", "dev");
    r.commit("work", "c.txt", "c\n", "and another");
    assert_eq!(git.run(&Op::Push(plan)), Err("dev moved since this was planned: plan it again".into()));
    assert_eq!(r.rev("origin", "dev"), remote);

    r.sh("work", &["push", "-q"]);
    r.sh("work", &["commit", "-q", "--amend", "-m", "amended"]);
    let plan = git.push_plan().unwrap();
    assert!(matches!(plan, PushPlan::Replace { .. }), "{plan:?}");
    let remote = r.rev("origin", "dev");
    r.sh("work", &["commit", "-q", "--amend", "-m", "amended again"]);
    assert_eq!(git.run(&Op::Push(plan)), Err("dev moved since this was planned: plan it again".into()));
    assert_eq!(r.rev("origin", "dev"), remote);
    assert_eq!(r.sh("work", &["for-each-ref", "refs/devdeck/"]), "", "no backup ref for a push not made");
}

#[test]
fn fetches_never_prune_local_tags() {
    let r = Repos::new("prune-tags");
    r.on_dev();
    r.sh("work", &["config", "fetch.pruneTags", "true"]);
    r.sh("work", &["tag", "mine"]);
    let git = r.git("work");
    git.run(&Op::Fetch).unwrap();
    git.run(&Op::Pull("dev".into())).unwrap();
    git.push_plan().unwrap();
    assert!(git.has_ref("mine"));
}

#[test]
fn one_write_at_a_time_per_repository() {
    let r = Repos::new("one-write");
    std::os::unix::fs::symlink(r.path("work"), r.path("link")).unwrap();
    let started = r.path("hook-started");
    r.hook("work", "post-checkout", &format!("touch '{}'\nsleep 2", started.display()));
    let (first, second) = (r.git("work"), r.git("link"));
    let slow = std::thread::spawn(move || first.run(&Op::Create("slow".into())));
    let start = Instant::now();
    while !started.exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "the hook never ran");
        std::thread::sleep(Duration::from_millis(20));
    }
    let busy = "another git operation is running in this repository".to_string();
    assert_eq!(second.run(&Op::Fetch), Err(busy.clone()));
    assert_eq!(second.push_plan(), Err(busy));
    slow.join().unwrap().unwrap();
    second.run(&Op::Fetch).unwrap();
}

#[test]
fn a_cherry_pick_sequence_between_picks_blocks_writes_until_aborted() {
    let r = Repos::new("sequencer");
    r.commit("work", "a.txt", "main\n", "main");
    r.sh("work", &["switch", "-q", "-c", "side", "HEAD~1"]);
    r.commit("work", "a.txt", "side\n", "side 1");
    r.commit("work", "b.txt", "b\n", "side 2");
    r.sh("work", &["switch", "-q", "main"]);
    assert!(!r.run("work", &["cherry-pick", "side~1", "side"]).status.success());
    r.write("work/a.txt", "resolved\n");
    r.sh("work", &["commit", "-q", "-a", "-m", "side 1 resolved"]);
    assert!(!r.path("work/.git/CHERRY_PICK_HEAD").exists() && r.path("work/.git/sequencer").exists());
    let git = r.git("work");
    assert_eq!(git.status().unwrap().doing, Some(Doing::CherryPicking));
    assert_eq!(git.run(&Op::Fetch), Err("a cherry-pick is under way: finish it, or abort".into()));
    git.run(&Op::Abort(Doing::CherryPicking)).unwrap();
    assert!(!r.path("work/.git/sequencer").exists());
    assert_eq!(git.writes_blocked(), Ok(None));
}

#[test]
fn a_stopped_am_is_shown_and_aborted() {
    let r = Repos::new("am");
    r.sh("work", &["switch", "-q", "-c", "side"]);
    r.commit("work", "a.txt", "side\n", "side");
    r.sh("work", &["format-patch", "-q", "-1", "-o", "../patches"]);
    r.sh("work", &["switch", "-q", "main"]);
    r.commit("work", "a.txt", "main\n", "main");
    let start = r.rev("work", "HEAD");
    let patch = std::fs::read_dir(r.path("patches")).unwrap().next().unwrap().unwrap().path();
    assert!(!r.run("work", &["am", patch.to_str().unwrap()]).status.success());
    let git = r.git("work");
    assert_eq!(git.status().unwrap().doing, Some(Doing::Applying));
    assert_eq!(git.run(&Op::Switch("side".into())), Err("a git am is under way: finish it, or abort".into()));
    assert_eq!(git.run(&Op::Abort(Doing::Rebasing)), Err("no rebase under way".into()));
    git.run(&Op::Abort(Doing::Applying)).unwrap();
    assert!(!r.path("work/.git/rebase-apply").exists());
    assert_eq!(r.rev("work", "HEAD"), start);
}

#[test]
fn an_abort_lists_the_tracked_changes_it_resets() {
    let r = Repos::new("abort-impact");
    r.commit("work", "u.txt", "u\n", "unrelated");
    r.commit("work", "a.txt", "theirs\n", "theirs");
    r.sh("work", &["switch", "-q", "-c", "side", "HEAD~1"]);
    r.commit("work", "a.txt", "ours\n", "ours");
    let git = r.git("work");
    assert_eq!(git.abort_impact(), Ok(vec![]), "nothing under way");
    assert!(!r.run("work", &["rebase", "main"]).status.success());
    r.write("work/u.txt", "edited meanwhile\n");
    r.write("work/new.txt", "untracked: left alone\n");
    assert_eq!(git.abort_impact(), Ok(vec!["a.txt".to_string(), "u.txt".to_string()]));
}

#[test]
fn a_switch_carries_changes_that_don_t_conflict() {
    let r = Repos::new("carry");
    r.sh("work", &["switch", "-q", "-c", "other"]);
    r.commit("work", "b.txt", "b\n", "other");
    r.sh("work", &["switch", "-q", "main"]);
    r.write("work/a.txt", "local edit\n");
    let git = r.git("work");
    git.run(&Op::Switch("other".into())).unwrap();
    assert_eq!(git.head(), Ok(Head::Branch("other".into())));
    assert_eq!(r.read("work/a.txt"), "local edit\n");
}

#[test]
fn an_untracked_file_in_the_way_is_never_overwritten() {
    let r = Repos::new("untracked-in-the-way");
    r.sh("work", &["switch", "-q", "-c", "other"]);
    r.commit("work", "n.txt", "committed\n", "other");
    r.sh("work", &["switch", "-q", "main"]);
    r.write("work/n.txt", "mine\n");
    let git = r.git("work");
    let e = git.run(&Op::Switch("other".into())).unwrap_err();
    assert!(e.contains("untracked working tree files would be overwritten"), "{e}");
    assert_eq!(r.read("work/n.txt"), "mine\n");
    assert_eq!(git.head(), Ok(Head::Branch("main".into())));
}
