//! A repository through the `git` program (R6.10): its state and branches, and the few
//! operations devdeck makes (switch, new branch, fetch, pull fast-forward only, a push
//! that never loses anything, abort an operation under way). Never forced; git's own
//! message when it refuses. See docs/plans/M3-cluster.md, step 3.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// A repository, run with the login shell's variables.
#[derive(Clone, Debug)]
pub struct Git {
    root: PathBuf,
    /// This worktree's git dir: what is under way (`rebase-merge`, `MERGE_HEAD`…).
    git_dir: PathBuf,
    /// The common git dir, resolved: one write at a time per repository.
    common_dir: PathBuf,
    vars: Vec<(String, String)>,
}

/// Where a repository's files are (a worktree has its own git dir and a common one).
#[derive(Clone, Debug, PartialEq)]
pub struct GitDirs {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Head {
    Branch(String),
    /// At a commit (short id), not on a branch.
    Detached(String),
    /// A branch without a commit yet.
    Unborn(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Upstream {
    pub name: String,
    pub ahead: u32,
    pub behind: u32,
    /// Configured, but gone from the remote.
    pub gone: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    /// A rename's or copy's original path.
    pub from: Option<String>,
    pub kind: ChangeKind,
    pub staged: bool,
}

/// An operation under way in the repository (from its git dir).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Doing {
    Rebasing,
    /// `git am` stopped on a patch.
    Applying,
    Merging,
    CherryPicking,
    Reverting,
    Bisecting,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub head: Head,
    pub upstream: Option<Upstream>,
    pub changes: Vec<Change>,
    pub doing: Option<Doing>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    /// `main`, or `origin/main` for a remote one.
    pub name: String,
    /// The remote of a remote branch.
    pub remote: Option<String>,
    /// A local branch's upstream (`origin/main`).
    pub upstream: Option<String>,
    /// Short id.
    pub commit: String,
    pub subject: String,
    pub author: String,
    /// Committer date, epoch seconds.
    pub at: u64,
    pub current: bool,
    /// Checked out in another worktree (switching to it is refused).
    pub worktree: Option<PathBuf>,
}

/// A commit as listed.
#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    /// Full id.
    pub id: String,
    pub subject: String,
    pub author: String,
}

/// What devdeck asks git to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Fetch,
    /// `fetch`, then a fast-forward of this branch (the current one) to its upstream.
    Pull(String),
    Switch(String),
    /// A remote branch (`origin/x`): a local branch tracking it, switched to.
    Track(String),
    /// A new branch from the current commit, switched to.
    Create(String),
    /// The push [`Git::push_plan`] decided (not `Behind` or `UpToDate`).
    Push(PushPlan),
    /// Abandons the operation under way, back to where it started.
    Abort(Doing),
}

/// What a push would do, decided on the remote's state just fetched.
#[derive(Clone, Debug, PartialEq)]
pub enum PushPlan {
    /// The branch isn't on the remote: published, and tracked from then on.
    Publish { remote: String, branch: String },
    /// The remote's tip is in the branch: a plain push of `commits` commits, up to `head`
    /// (HEAD's full id when planned).
    Send { remote: String, branch: String, head: String, commits: u32 },
    /// The branch was rewritten: the remote's tip is in its reflog, and each of the
    /// `replaced` commits (the remote's own, newest first) is the user's or has an
    /// equivalent in the branch. Replaced by `head`, with a lease on `expected` (the remote
    /// tip fetched), after a backup ref `refs/devdeck/replaced/<branch>/<time>` at it.
    Replace { remote: String, branch: String, expected: String, head: String, replaced: Vec<Commit> },
    /// The remote has `commits` commits the branch never had: refused.
    Behind { remote: String, branch: String, commits: u32 },
    UpToDate { remote: String, branch: String },
}

impl Git {
    /// The repository holding `dir`, looked for up to `ceiling` (the workspace folder: a
    /// repository there opens, one above it doesn't).
    pub fn open(dir: &Path, ceiling: Option<&Path>, vars: Vec<(String, String)>) -> Result<(Git, GitDirs), String> {
        let args = ["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-dir", "--git-common-dir"];
        // git doesn't go up into a ceiling directory, so the ceiling is the folder's parent.
        let out = exec(dir, &vars, ceiling.and_then(Path::parent), &args, Mode::Read)?;
        if !out.ok {
            return Err(out.failure(&args));
        }
        let text = out.text();
        let mut lines = text.lines().map(PathBuf::from);
        let (Some(root), Some(git_dir), Some(common_dir)) = (lines.next(), lines.next(), lines.next()) else {
            return Err(format!("{}: not a repository with a working tree", dir.display()));
        };
        let resolved = std::fs::canonicalize(&common_dir).unwrap_or_else(|_| common_dir.clone());
        let dirs = GitDirs { root: root.clone(), git_dir: git_dir.clone(), common_dir };
        Ok((Git { root, git_dir, common_dir: resolved, vars }, dirs))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn status(&self) -> Result<Status, String> {
        let out = self.read(&["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=normal"])?;
        let mut status = parse_status(&out);
        if let Head::Detached(_) = status.head {
            status.head = Head::Detached(self.read_text(&["rev-parse", "--short", "HEAD"])?);
        }
        status.doing = self.doing();
        Ok(status)
    }

    pub fn branches(&self) -> Result<Vec<Branch>, String> {
        let format = ["%(refname)", "%(upstream:short)", "%(objectname:short)", "%(authorname)", "%(committerdate:unix)", "%(HEAD)", "%(symref)", "%(worktreepath)", "%(contents:subject)"].join("%00");
        let out = self.read_text(&["for-each-ref", &format!("--format={format}"), "refs/heads", "refs/remotes"])?;
        let mut branches: Vec<Branch> = out.lines().filter_map(|line| self.branch(line)).collect();
        branches.sort_by(|a, b| (a.remote.is_some(), &a.name).cmp(&(b.remote.is_some(), &b.name)));
        Ok(branches)
    }

    /// The branch checked out (cheaper than a status).
    pub fn head(&self) -> Result<Head, String> {
        let args = ["symbolic-ref", "-q", "--short", "HEAD"];
        let out = self.exec(&args, Mode::Read)?;
        if out.ok {
            let name = out.text().trim().to_string();
            return Ok(if self.commit_of("HEAD")?.is_some() { Head::Branch(name) } else { Head::Unborn(name) });
        }
        if out.code != Some(1) {
            return Err(out.failure(&args));
        }
        Ok(Head::Detached(self.read_text(&["rev-parse", "--short", "HEAD"])?))
    }

    /// Whether `name` is a branch, remote branch or tag.
    pub fn has_ref(&self, name: &str) -> bool {
        ["refs/heads", "refs/remotes", "refs/tags"].iter().any(|kind| self.has(&format!("{kind}/{name}")))
    }

    /// Runs `op`; git's message when it fails.
    pub fn run(&self, op: &Op) -> Result<String, String> {
        let _writing = self.writing()?;
        if let Op::Abort(doing) = op {
            return self.abort(*doing);
        }
        if let Some(why) = self.writes_blocked()? {
            return Err(why);
        }
        match op {
            Op::Fetch => self.write(&FETCH, Mode::Network(FETCH_LIMIT)),
            Op::Pull(branch) => self.pull(branch),
            Op::Switch(branch) => {
                plain(branch)?;
                self.write(&["switch", "--no-overwrite-ignore", "--no-guess", "--", branch], Mode::Tree)
            }
            Op::Track(remote_branch) => {
                plain(remote_branch)?;
                if !self.has(&format!("refs/remotes/{remote_branch}")) {
                    return Err(format!("{remote_branch} isn't a remote branch"));
                }
                self.write(&["switch", "--no-overwrite-ignore", "--track", "--", remote_branch], Mode::Tree)
            }
            Op::Create(name) => {
                self.check_branch_name(name)?;
                self.write(&["switch", "--no-overwrite-ignore", "-c", name], Mode::Tree)
            }
            Op::Push(plan) => self.push(plan),
            Op::Abort(_) => unreachable!("handled above"),
        }
    }

    /// Fetches the push remote, then decides what a push of the current branch would do.
    pub fn push_plan(&self) -> Result<PushPlan, String> {
        let _writing = self.writing()?;
        if let Some(why) = self.writes_blocked()? {
            return Err(why);
        }
        let branch = match self.head()? {
            Head::Branch(b) => b,
            Head::Detached(commit) => return Err(format!("detached at {commit}: no branch to push")),
            Head::Unborn(b) => return Err(format!("{b} has no commit yet")),
        };
        plain(&branch)?;
        let remote = self.push_remote(&branch)?;
        plain(&remote)?;
        self.write(&[&FETCH[..], &["--", &remote]].concat(), Mode::Network(FETCH_LIMIT))?;
        let Some(tip) = self.commit_of(&format!("refs/remotes/{remote}/{branch}"))? else {
            return Ok(PushPlan::Publish { remote, branch });
        };
        let head = self.commit_of("HEAD")?.ok_or("HEAD has no commit")?;
        if tip == head {
            return Ok(PushPlan::UpToDate { remote, branch });
        }
        if self.is_ancestor(&tip, &head)? {
            let commits = self.count(&format!("{tip}..{head}"))?;
            return Ok(PushPlan::Send { remote, branch, head, commits });
        }
        let remote_only = self.remote_only(&head, &tip)?;
        let commits = remote_only.len() as u32;
        if self.is_ancestor(&head, &tip)? || !self.in_reflog(&branch, &tip)? {
            return Ok(PushPlan::Behind { remote, branch, commits });
        }
        let me = self.author_email()?;
        let kept = remote_only.iter().all(|c| c.equivalent || me.as_deref().is_some_and(|me| c.email.eq_ignore_ascii_case(me)));
        if !kept {
            return Ok(PushPlan::Behind { remote, branch, commits });
        }
        let replaced = remote_only.into_iter().map(|c| c.commit).collect();
        Ok(PushPlan::Replace { remote, branch, expected: tip, head, replaced })
    }

    /// The tracked files whose changes an abort of the operation under way would reset
    /// (a rebase's abort resets hard; a merge's, cherry-pick's or `am`'s may). Nothing for
    /// a bisect or when nothing is under way.
    pub fn abort_impact(&self) -> Result<Vec<String>, String> {
        if matches!(self.doing(), None | Some(Doing::Bisecting)) {
            return Ok(Vec::new());
        }
        let mut files: Vec<String> = self.status()?.changes.into_iter().filter(|c| c.kind != ChangeKind::Untracked).map(|c| c.path).collect();
        files.sort();
        files.dedup();
        Ok(files)
    }

    /// Why nothing but an abort can be done now (an operation under way), if so.
    pub fn writes_blocked(&self) -> Result<Option<String>, String> {
        Ok(self.doing().map(|d| format!("{} is under way: finish it, or abort", noun(d))))
    }

    /// Whether `name` can be a new branch's.
    pub fn check_branch_name(&self, name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("a branch needs a name".into());
        }
        plain(name)?;
        if name == "@" || name == "HEAD" {
            return Err(format!("{name} can't be a branch name"));
        }
        let args = ["check-ref-format", &format!("refs/heads/{name}")];
        if !self.exec(&args, Mode::Read)?.ok {
            return Err(format!("{name} isn't a valid branch name"));
        }
        if self.has(&format!("refs/heads/{name}")) {
            return Err(format!("branch {name} exists"));
        }
        Ok(())
    }

    fn exec(&self, args: &[&str], mode: Mode) -> Result<Output, String> {
        exec(&self.root, &self.vars, None, args, mode)
    }

    /// This repository's write slot, process-wide; refused while another write holds it.
    fn writing(&self) -> Result<Writing, String> {
        let mut writing = WRITING.lock().unwrap_or_else(|e| e.into_inner());
        if !writing.insert(self.common_dir.clone()) {
            return Err("another git operation is running in this repository".into());
        }
        Ok(Writing(self.common_dir.clone()))
    }

    /// Whether HEAD is still `branch` (at `commit`, when given), as when the operation was
    /// planned.
    fn still(&self, branch: &str, commit: Option<&str>) -> Result<(), String> {
        if self.head()? != Head::Branch(branch.to_string()) {
            return Err(format!("{branch} isn't the current branch any more"));
        }
        if let Some(commit) = commit
            && self.commit_of("HEAD")?.as_deref() != Some(commit)
        {
            return Err(format!("{branch} moved since this was planned: plan it again"));
        }
        Ok(())
    }

    /// The commits of `tip` that `head` doesn't have, newest first, each with its author's
    /// email and whether `head` has an equivalent (the same patch) of it.
    fn remote_only(&self, head: &str, tip: &str) -> Result<Vec<RemoteCommit>, String> {
        let range = format!("{head}...{tip}");
        let format = "--format=%m%x00%H%x00%ae%x00%an%x00%s";
        let out = self.read_text(&["log", "--no-show-signature", "--no-color", "--topo-order", "--cherry-mark", "--right-only", format, &range, "--"])?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let [mark, id, email, author, subject] = line.splitn(5, '\0').collect::<Vec<_>>()[..] else { return None };
                let commit = Commit { id: id.into(), subject: subject.into(), author: author.into() };
                Some(RemoteCommit { commit, email: email.into(), equivalent: mark == "=" })
            })
            .collect())
    }

    /// The email git would author commits with (config or environment), if any.
    fn author_email(&self) -> Result<Option<String>, String> {
        let out = self.exec(&["var", "GIT_AUTHOR_IDENT"], Mode::Read)?;
        let ident = out.text();
        Ok(out.ok.then(|| ident.split_once('<').and_then(|(_, rest)| rest.split_once('>')).map(|(email, _)| email.trim().to_string())).flatten().filter(|e| !e.is_empty()))
    }

    /// A read's output; git's message when it fails.
    fn read(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        let out = self.exec(args, Mode::Read)?;
        if out.ok { Ok(out.stdout) } else { Err(out.failure(args)) }
    }

    fn read_text(&self, args: &[&str]) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.read(args)?).trim().to_string())
    }

    /// A write's output (stdout, then stderr); git's message when it fails.
    fn write(&self, args: &[&str], mode: Mode) -> Result<String, String> {
        let out = self.exec(args, mode)?;
        if !out.ok {
            return Err(out.failure(args));
        }
        let both = [out.text().trim().to_string(), String::from_utf8_lossy(&out.stderr).trim().to_string()];
        Ok(both.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"))
    }

    /// Whether the full ref `name` exists.
    fn has(&self, name: &str) -> bool {
        !name.split('/').any(|s| s.starts_with('-')) && self.exec(&["show-ref", "--verify", "--quiet", name], Mode::Read).is_ok_and(|o| o.ok)
    }

    /// The commit `rev` names, `None` when it doesn't exist.
    fn commit_of(&self, rev: &str) -> Result<Option<String>, String> {
        let args = ["rev-parse", "-q", "--verify", &format!("{rev}^{{commit}}")];
        let out = self.exec(&args, Mode::Read)?;
        match (out.ok, out.code) {
            (true, _) => Ok(Some(out.text().trim().to_string())),
            (false, Some(1)) => Ok(None),
            _ => Err(out.failure(&args)),
        }
    }

    fn is_ancestor(&self, ancestor: &str, of: &str) -> Result<bool, String> {
        let args = ["merge-base", "--is-ancestor", ancestor, of];
        let out = self.exec(&args, Mode::Read)?;
        match (out.ok, out.code) {
            (true, _) => Ok(true),
            (false, Some(1)) => Ok(false),
            _ => Err(out.failure(&args)),
        }
    }

    /// How many commits `range` holds.
    fn count(&self, range: &str) -> Result<u32, String> {
        let n = self.read_text(&["rev-list", "--count", range, "--"])?;
        n.parse().map_err(|_| format!("rev-list --count gave {n}"))
    }

    /// Whether `commit` is in the history of one of `branch`'s reflog entries (the newest
    /// [`REFLOG_ENTRIES`]): the branch had it before a rebase or an amend.
    fn in_reflog(&self, branch: &str, commit: &str) -> Result<bool, String> {
        let limit = REFLOG_ENTRIES.to_string();
        let out = self.exec(&["reflog", "show", "--format=%H", "-n", &limit, &format!("refs/heads/{branch}"), "--"], Mode::Read)?;
        let text = out.text();
        let entries: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        if !out.ok || entries.is_empty() {
            return Ok(false);
        }
        // Nothing of `commit`'s history is left once every entry's history is taken out
        // exactly when one of them holds it.
        let mut args = vec!["rev-list", "--count", commit, "--not"];
        args.extend(entries);
        Ok(self.read_text(&args)? == "0")
    }

    /// Where `branch` is pushed: its upstream's remote, its `pushRemote`,
    /// `remote.pushDefault`, else the only remote.
    fn push_remote(&self, branch: &str) -> Result<String, String> {
        for key in [format!("branch.{branch}.remote"), format!("branch.{branch}.pushRemote"), "remote.pushDefault".into()] {
            if let Some(remote) = self.config(&key)?.filter(|r| r != ".") {
                return Ok(remote);
            }
        }
        let remotes = self.read_text(&["remote"])?;
        let remotes: Vec<&str> = remotes.lines().collect();
        match remotes[..] {
            [] => Err("no remote to push to".into()),
            [one] => Ok(one.to_string()),
            _ => Err(format!("several remotes ({}) and none chosen for {branch}: set its upstream or remote.pushDefault", remotes.join(", "))),
        }
    }

    /// A config value, `None` when unset.
    fn config(&self, key: &str) -> Result<Option<String>, String> {
        let args = ["config", "--get", key];
        let out = self.exec(&args, Mode::Read)?;
        match (out.ok, out.code) {
            (true, _) => Ok(Some(out.text().trim().to_string())),
            (false, Some(1)) => Ok(None),
            _ => Err(out.failure(&args)),
        }
    }

    fn pull(&self, branch: &str) -> Result<String, String> {
        plain(branch)?;
        self.still(branch, None)?;
        if !self.exec(&["rev-parse", "-q", "--verify", "--symbolic-full-name", "@{u}"], Mode::Read)?.ok {
            return Err("no upstream to pull from".into());
        }
        let fetched = self.write(&FETCH, Mode::Network(FETCH_LIMIT))?;
        self.still(branch, None)?;
        let merged = self.write(&["merge", "--ff-only", "--no-autostash", "--no-overwrite-ignore", "@{u}"], Mode::Tree)?;
        Ok([fetched, merged].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"))
    }

    fn push(&self, plan: &PushPlan) -> Result<String, String> {
        let (remote, branch) = match plan {
            PushPlan::Behind { remote, branch, commits } => {
                return Err(format!("{remote} has {commits} commit(s) {branch} never had: pull them first, or handle it in a terminal"));
            }
            PushPlan::UpToDate { remote, branch } => return Err(format!("{branch} is up to date on {remote}: nothing to push")),
            PushPlan::Publish { remote, branch } | PushPlan::Send { remote, branch, .. } | PushPlan::Replace { remote, branch, .. } => (remote, branch),
        };
        plain(remote)?;
        plain(branch)?;
        let limit = Mode::Network(PUSH_LIMIT);
        match plan {
            PushPlan::Publish { .. } => {
                self.still(branch, None)?;
                let spec = format!("refs/heads/{branch}:refs/heads/{branch}");
                self.write(&["push", "--set-upstream", "--", remote, &spec], limit)
            }
            PushPlan::Send { head, .. } => {
                full_id(head)?;
                self.still(branch, Some(head))?;
                self.write(&["push", "--", remote, &format!("{head}:refs/heads/{branch}")], limit)
            }
            PushPlan::Replace { expected, head, .. } => {
                full_id(expected)?;
                full_id(head)?;
                self.still(branch, Some(head))?;
                // The replaced tip stays reachable, whatever happens next.
                let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let backup = format!("refs/devdeck/replaced/{branch}/{at}");
                self.write(&["update-ref", "-m", "devdeck: before replacing it on the remote", &backup, expected, ""], Mode::Tree)?;
                let lease = format!("--force-with-lease=refs/heads/{branch}:{expected}");
                self.write(&["push", &lease, "--force-if-includes", "--", remote, &format!("{head}:refs/heads/{branch}")], limit)
            }
            PushPlan::Behind { .. } | PushPlan::UpToDate { .. } => unreachable!("refused above"),
        }
    }

    fn abort(&self, doing: Doing) -> Result<String, String> {
        if self.doing() != Some(doing) {
            return Err(format!("no {} under way", noun(doing).trim_start_matches("a ")));
        }
        let args: &[&str] = match doing {
            Doing::Rebasing => &["rebase", "--abort"],
            Doing::Applying => &["am", "--abort"],
            Doing::Merging => &["merge", "--abort"],
            Doing::CherryPicking => &["cherry-pick", "--abort"],
            Doing::Reverting => &["revert", "--abort"],
            Doing::Bisecting => &["bisect", "reset"],
        };
        self.write(args, Mode::Tree)
    }

    /// The operation under way, from the git dir.
    fn doing(&self) -> Option<Doing> {
        let at = |name: &str| self.git_dir.join(name).exists();
        if at("rebase-merge") {
            Some(Doing::Rebasing)
        } else if at("rebase-apply") {
            // `am` and the old rebase backend share the folder; `am` marks it.
            Some(if at("rebase-apply/applying") { Doing::Applying } else { Doing::Rebasing })
        } else if at("MERGE_HEAD") {
            Some(Doing::Merging)
        } else if at("CHERRY_PICK_HEAD") {
            Some(Doing::CherryPicking)
        } else if at("REVERT_HEAD") {
            Some(Doing::Reverting)
        } else if at("sequencer") {
            // A sequence of picks or reverts, between two of them (a conflict resolved
            // with `git commit`): its next command says which.
            let todo = std::fs::read_to_string(self.git_dir.join("sequencer/todo")).unwrap_or_default();
            let next = todo.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).and_then(|l| l.split_whitespace().next());
            Some(if next == Some("revert") { Doing::Reverting } else { Doing::CherryPicking })
        } else if at("BISECT_LOG") {
            Some(Doing::Bisecting)
        } else {
            None
        }
    }

    /// One `for-each-ref` line; `None` for a remote's `HEAD`.
    fn branch(&self, line: &str) -> Option<Branch> {
        let f: Vec<&str> = line.splitn(9, '\0').collect();
        let [refname, upstream, commit, author, at, head, symref, worktree, subject] = f[..] else { return None };
        if !symref.is_empty() {
            return None;
        }
        let (name, remote) = match (refname.strip_prefix("refs/heads/"), refname.strip_prefix("refs/remotes/")) {
            (Some(name), _) => (name, None),
            (_, Some(name)) => (name, name.split('/').next().map(String::from)),
            _ => return None,
        };
        let worktree = Some(PathBuf::from(worktree)).filter(|w| !w.as_os_str().is_empty() && !same_path(w, &self.root));
        Some(Branch {
            name: name.to_string(),
            remote,
            upstream: Some(upstream.to_string()).filter(|u| !u.is_empty()),
            commit: commit.to_string(),
            subject: subject.to_string(),
            author: author.to_string(),
            at: at.parse().unwrap_or(0),
            current: head == "*",
            worktree,
        })
    }
}

/// How long a read may take.
const READ_LIMIT: Duration = Duration::from_secs(10);
/// How long a fetch may take (a hung remote, a network gone).
const FETCH_LIMIT: Duration = Duration::from_secs(60);
/// A push may run a long pre-push hook (tests); stopping it before the ref update is safe.
const PUSH_LIMIT: Duration = Duration::from_secs(600);
/// Between SIGTERM and SIGKILL when git took too long.
const GRACE: Duration = Duration::from_secs(5);
/// How long output is still read after git exits (a helper it started may hold the pipes).
const AFTER_EXIT: Duration = Duration::from_secs(1);
/// How many reflog entries a push looks into for the remote's tip.
const REFLOG_ENTRIES: usize = 200;
/// Every fetch: remote branches gone are pruned, local tags never (`fetch.pruneTags`).
const FETCH: [&str; 3] = ["fetch", "--prune", "--no-prune-tags"];

/// The repositories (common git dirs) a write runs in, process-wide.
static WRITING: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// A repository's write slot, given back when dropped.
struct Writing(PathBuf);

impl Drop for Writing {
    fn drop(&mut self) {
        WRITING.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

/// A commit of the remote's that the branch doesn't have.
struct RemoteCommit {
    commit: Commit,
    email: String,
    /// The branch has the same patch.
    equivalent: bool,
}

/// How a run is bounded.
#[derive(Clone, Copy)]
enum Mode {
    /// Changes nothing: no optional locks, [`READ_LIMIT`].
    Read,
    /// Talks to a remote, touches no working tree: stopped after the limit.
    Network(Duration),
    /// Touches the working tree or refs: never stopped (a half switched tree is worse);
    /// its output goes to files, so nothing devdeck does (quitting) can break its pipes.
    Tree,
}

struct Output {
    ok: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Output {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// git's message, else what failed.
    fn failure(&self, args: &[&str]) -> String {
        let message = String::from_utf8_lossy(&self.stderr).trim().to_string();
        if !message.is_empty() {
            return message;
        }
        let code = self.code.map_or("a signal".to_string(), |c| c.to_string());
        format!("git {} failed (exit {code})", args.join(" "))
    }
}

#[cfg(test)]
thread_local! {
    /// Replaces every limit in this test thread.
    static TEST_LIMIT: std::cell::Cell<Option<Duration>> = const { std::cell::Cell::new(None) };
}

fn limit(d: Duration) -> Duration {
    #[cfg(test)]
    if let Some(test) = TEST_LIMIT.with(|t| t.get()) {
        return test;
    }
    d
}

/// `git -C <dir> <args>` in its own session (no terminal for ssh to prompt on), stdin
/// closed, with `vars` (devdeck's own environment when empty), never prompting, not
/// looking for a repository in `ceiling` or above. Err only when it can't start or took
/// too long.
fn exec(dir: &Path, vars: &[(String, String)], ceiling: Option<&Path>, args: &[&str], mode: Mode) -> Result<Output, String> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    if !vars.is_empty() {
        cmd.env_clear().envs(vars.iter().map(|(k, v)| (k, v)));
    }
    for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OPTIONAL_LOCKS", "GIT_CEILING_DIRECTORIES", "SSH_ASKPASS"] {
        cmd.env_remove(name);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0").env("SSH_ASKPASS_REQUIRE", "never");
    if let Some(ceiling) = ceiling {
        cmd.env("GIT_CEILING_DIRECTORIES", ceiling);
    }
    if let Mode::Read = mode {
        cmd.env("GIT_OPTIONAL_LOCKS", "0");
    }
    cmd.stdin(Stdio::null());
    // SAFETY: `setsid` is async-signal-safe and touches no memory of the parent.
    unsafe {
        cmd.pre_exec(|| rustix::process::setsid().map(|_| ()).map_err(Into::into));
    }
    if let Mode::Tree = mode {
        return to_files(cmd);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("git: {e}"))?;
    let (tx, rx) = mpsc::channel();
    let stdout = drain(child.stdout.take().expect("stdout is piped"), tx.clone());
    let stderr = drain(child.stderr.take().expect("stderr is piped"), tx);
    let deadline = match mode {
        Mode::Read => Some(Instant::now() + limit(READ_LIMIT)),
        Mode::Network(d) => Some(Instant::now() + limit(d)),
        Mode::Tree => None,
    };
    let mut closed = 0;
    let mut exited: Option<(Instant, ExitStatus)> = None;
    let status = loop {
        if closed == 2 {
            break match exited {
                Some((_, status)) => status,
                None => child.wait().map_err(|e| format!("git: {e}"))?,
            };
        }
        if rx.recv_timeout(Duration::from_millis(50)).is_ok() {
            closed += 1;
            continue;
        }
        if exited.is_none()
            && let Some(status) = child.try_wait().map_err(|e| format!("git: {e}"))?
        {
            exited = Some((Instant::now(), status));
        }
        match exited {
            Some((at, status)) if at.elapsed() >= AFTER_EXIT => break status,
            Some(_) => {}
            None if deadline.is_some_and(|d| Instant::now() >= d) => {
                stop(&mut child);
                return Err(format!("git {} took too long", args.first().unwrap_or(&"")));
            }
            None => {}
        }
    };
    let take = |buf: Arc<Mutex<Vec<u8>>>| std::mem::take(&mut *buf.lock().unwrap_or_else(|e| e.into_inner()));
    Ok(Output { ok: status.success(), code: status.code(), stdout: take(stdout), stderr: take(stderr) })
}

/// Runs `cmd` to its end, its output in temporary files, read afterwards.
fn to_files(mut cmd: Command) -> Result<Output, String> {
    static N: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!("devdeck-git-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let (out_path, err_path) = (base.with_extension("out"), base.with_extension("err"));
    let create = |path: &Path| {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::remove_file(path);
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|e| format!("{}: {e}", path.display()))
    };
    let status = (|| {
        cmd.stdout(create(&out_path)?).stderr(create(&err_path)?);
        cmd.status().map_err(|e| format!("git: {e}"))
    })();
    let stdout = std::fs::read(&out_path).unwrap_or_default();
    let stderr = std::fs::read(&err_path).unwrap_or_default();
    let _ = std::fs::remove_file(&out_path);
    let _ = std::fs::remove_file(&err_path);
    let status = status?;
    Ok(Output { ok: status.success(), code: status.code(), stdout, stderr })
}

/// Reads `pipe` on a thread to its end, then says so on `done`.
fn drain(mut pipe: impl Read + Send + 'static, done: mpsc::Sender<()>) -> Arc<Mutex<Vec<u8>>> {
    let out = Arc::new(Mutex::new(Vec::new()));
    let sink = out.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n @ 1..) = pipe.read(&mut buf) {
            sink.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&buf[..n]);
        }
        let _ = done.send(());
    });
    out
}

/// SIGTERM to git's process group, SIGKILL after [`GRACE`].
fn stop(child: &mut Child) {
    use rustix::process::{Pid, Signal, kill_process_group};
    let group = Pid::from_child(child);
    let _ = kill_process_group(group, Signal::TERM);
    let end = Instant::now() + GRACE;
    while Instant::now() < end && matches!(child.try_wait(), Ok(None)) {
        std::thread::sleep(Duration::from_millis(20));
    }
    // What git started (ssh) may outlive it.
    let _ = kill_process_group(group, Signal::KILL);
    let _ = child.wait();
}

/// Refuses anything but a full commit id.
fn full_id(id: &str) -> Result<(), String> {
    if id.len() >= 40 && id.chars().all(|c| c.is_ascii_hexdigit()) { Ok(()) } else { Err(format!("{id:?} isn't a full commit id")) }
}

/// Refuses a name git could read as an option.
fn plain(name: &str) -> Result<(), String> {
    if name.is_empty() {
        Err("a name is needed".into())
    } else if name.starts_with('-') {
        Err(format!("{name}: a name can't start with -"))
    } else {
        Ok(())
    }
}

fn noun(doing: Doing) -> &'static str {
    match doing {
        Doing::Rebasing => "a rebase",
        Doing::Applying => "a git am",
        Doing::Merging => "a merge",
        Doing::CherryPicking => "a cherry-pick",
        Doing::Reverting => "a revert",
        Doing::Bisecting => "a bisect",
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

/// `status --porcelain=v2 --branch -z`; a detached head holds the full commit id.
fn parse_status(out: &[u8]) -> Status {
    let mut oid = String::new();
    let mut head = String::new();
    let mut upstream: Option<String> = None;
    let mut ab: Option<(u32, u32)> = None;
    let mut changes = Vec::new();
    let mut fields = out.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).into_owned());
    while let Some(field) = fields.next() {
        if let Some(header) = field.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" => oid = value.to_string(),
                "branch.head" => head = value.to_string(),
                "branch.upstream" => upstream = Some(value.to_string()),
                "branch.ab" => {
                    let mut n = value.split(' ').map(|v| v.trim_start_matches(['+', '-']).parse().unwrap_or(0));
                    ab = Some((n.next().unwrap_or(0), n.next().unwrap_or(0)));
                }
                _ => {}
            }
        } else if let Some(rest) = field.strip_prefix("1 ") {
            // XY sub mH mI mW hH hI path
            if let [xy, .., path] = rest.splitn(8, ' ').collect::<Vec<_>>()[..] {
                changes.push(change(xy, path, None));
            }
        } else if let Some(rest) = field.strip_prefix("2 ") {
            // XY sub mH mI mW hH hI Xscore path, then the original path as its own field.
            let from = fields.next();
            if let [xy, .., path] = rest.splitn(9, ' ').collect::<Vec<_>>()[..] {
                changes.push(change(xy, path, from));
            }
        } else if let Some(rest) = field.strip_prefix("u ") {
            // XY sub m1 m2 m3 mW h1 h2 h3 path
            if let [_, .., path] = rest.splitn(10, ' ').collect::<Vec<_>>()[..] {
                changes.push(Change { path: path.to_string(), from: None, kind: ChangeKind::Conflicted, staged: false });
            }
        } else if let Some(path) = field.strip_prefix("? ") {
            changes.push(Change { path: path.to_string(), from: None, kind: ChangeKind::Untracked, staged: false });
        }
    }
    let head = if head == "(detached)" {
        Head::Detached(oid)
    } else if oid == "(initial)" {
        Head::Unborn(head)
    } else {
        Head::Branch(head)
    };
    let upstream = upstream.map(|name| {
        let (ahead, behind) = ab.unwrap_or((0, 0));
        Upstream { name, ahead, behind, gone: ab.is_none() }
    });
    Status { head, upstream, changes, doing: None }
}

/// A tracked file's change from its `XY`: the index's when something is staged, else the
/// working tree's.
fn change(xy: &str, path: &str, from: Option<String>) -> Change {
    let mut c = xy.chars();
    let (x, y) = (c.next().unwrap_or('.'), c.next().unwrap_or('.'));
    let staged = x != '.';
    let kind = match if staged { x } else { y } {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        'C' => ChangeKind::Copied,
        'T' => ChangeKind::TypeChanged,
        _ => ChangeKind::Modified,
    };
    Change { path: path.to_string(), from, kind, staged }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    /// An empty repository on `main` and isolated variables (`extra` added).
    fn repo(tag: &str, extra: &[(&str, &str)]) -> (TempDir, Vec<(String, String)>) {
        let dir = TempDir::new(tag);
        let mut vars: Vec<(String, String)> = [
            ("PATH", std::env::var("PATH").unwrap_or_default()),
            ("HOME", dir.display().to_string()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("GIT_AUTHOR_NAME", "Ada".into()),
            ("GIT_AUTHOR_EMAIL", "ada@example.com".into()),
            ("GIT_COMMITTER_NAME", "Ada".into()),
            ("GIT_COMMITTER_EMAIL", "ada@example.com".into()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        vars.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        sh(&dir, &vars, &["init", "-q", "-b", "main", "."]);
        (dir, vars)
    }

    fn sh(dir: &Path, vars: &[(String, String)], args: &[&str]) {
        let out = exec(dir, vars, None, args, Mode::Tree).unwrap();
        assert!(out.ok, "{}", out.failure(args));
    }

    /// Runs `f` with every limit at `limit`.
    fn limited<T>(limit: Duration, f: impl FnOnce() -> T) -> T {
        TEST_LIMIT.with(|t| t.set(Some(limit)));
        let out = f();
        TEST_LIMIT.with(|t| t.set(None));
        out
    }

    #[test]
    fn a_fetch_that_never_answers_is_stopped() {
        // ssh that never answers.
        let (dir, vars) = repo("git-timeout", &[("GIT_SSH_COMMAND", "sleep 100;:")]);
        sh(&dir, &vars, &["remote", "add", "origin", "ssh://nowhere.invalid/x.git"]);
        let (git, dirs) = Git::open(&dir, None, vars).unwrap();
        let start = Instant::now();
        let e = limited(Duration::from_secs(1), || git.run(&Op::Fetch));
        assert_eq!(e, Err("git fetch took too long".into()));
        assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
        fn locks(dir: &Path, found: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let path = e.path();
                if path.is_dir() {
                    locks(&path, found);
                } else if path.extension().is_some_and(|x| x == "lock") {
                    found.push(path);
                }
            }
        }
        let mut found = Vec::new();
        locks(&dirs.git_dir, &mut found);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn an_operation_on_the_working_tree_is_never_stopped() {
        let (dir, vars) = repo("git-slow-switch", &[]);
        sh(&dir, &vars, &["commit", "-q", "--allow-empty", "-m", "first"]);
        let hook = dir.join(".git/hooks/post-checkout");
        std::fs::write(&hook, "#!/bin/sh\nsleep 1\necho hook done\n").unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let (git, _) = Git::open(&dir, None, vars).unwrap();
        let start = Instant::now();
        let out = limited(Duration::from_millis(50), || git.run(&Op::Create("slow".into())));
        assert!(out.as_ref().is_ok_and(|o| o.contains("hook done")), "{out:?}");
        assert!(start.elapsed() >= Duration::from_secs(1));
        assert_eq!(git.head(), Ok(Head::Branch("slow".into())));
    }
}
