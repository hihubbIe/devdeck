//! A git repository as views see it (R6.10): its state and branches as last read, the
//! operation running (a failed one says so in a toast). One per repository root and
//! workspace, shared by every view showing it (the services keep weak ones by root); an
//! operation's task holds it until git exits. The core allows one write at a time per
//! repository, whatever holds it.
//!
//! Read only while a view holding it is visible ([`Looking`]): when git's files change
//! (the app's watcher, [`crate::watcher`]) or after an operation; changes while nobody looks
//! only mark it stale, read when one looks again. One read at a time; changes during a read
//! read once more after it. Nothing runs while idle. A git panel needs the full state (the
//! working tree's changes); a service or app view only the branches (a light read, no scan
//! of the working tree). Every write goes through the core's [`Git::run`], which refuses
//! while an operation (a rebase…) is under way. A dangerous one ([`Op::dangerous`]) runs
//! only with the [`Confirmed`] of its modal ([`Repo::run_confirmed`], R5.6).

use crate::danger::Confirmed;
use devdeck_core::cluster::Remote;
use devdeck_core::git::{Branch, Git, GitDirs, Head, Op, PushPlan, Status};
use gpui::{Context, EventEmitter, Task};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The repository's state as last read.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// The full state; a light read has only the head (no upstream, changes or operation).
    pub status: Status,
    pub branches: Vec<Branch>,
    /// Why nothing but an abort can be done (an operation under way); a light read doesn't
    /// know.
    pub blocked: Option<String>,
    /// When the remote was last fetched (epoch seconds), if ever.
    pub fetched: Option<u64>,
    /// A full read (a git panel looks), not a light one.
    pub full: bool,
}

/// Held by a view while it's on screen: the repository is read while one lives.
pub struct Looking(#[allow(dead_code)] Rc<()>);

/// Its git dirs are known (the watcher follows them).
pub struct Opened;

pub struct Repo {
    pub root: PathBuf,
    git: Option<Git>,
    pub dirs: Option<GitDirs>,
    pub state: Remote<Snapshot>,
    /// The operation running, in words ("fetching…"), and since when.
    pub busy: Option<(String, Instant)>,
    /// A push plan asked for and just decided, for the view that asked.
    pub plan: Option<Result<PushPlan, String>>,
    /// Views wanting the full state, and those wanting the branches only.
    full: Rc<()>,
    light: Rc<()>,
    reading: Option<Task<()>>,
    /// Changed while reading: read once more.
    again: bool,
    /// Changed while nobody looked.
    stale: bool,
    opening: Option<Task<()>>,
    /// Redraws the elapsed time while an operation runs.
    ticking: Option<Task<()>>,
}

impl EventEmitter<Opened> for Repo {}

impl Repo {
    /// The repository at `root`, opened off the UI thread with the login shell's `vars`;
    /// git looks no higher than `ceiling` (the workspace folder).
    pub fn new(root: PathBuf, ceiling: Option<PathBuf>, vars: Vec<(String, String)>, cx: &mut Context<Self>) -> Self {
        let dir = root.clone();
        let opening = cx.spawn(async move |this, cx| {
            let opened = cx.background_executor().spawn(async move { Git::open(&dir, ceiling.as_deref(), vars) }).await;
            let _ = this.update(cx, |r, cx| {
                r.opening = None;
                match opened {
                    Ok((git, dirs)) => {
                        r.git = Some(git);
                        r.dirs = Some(dirs);
                        cx.emit(Opened);
                        r.stale = true;
                        if r.looked() {
                            r.refresh(cx);
                        }
                    }
                    Err(e) => r.state = Remote::Error(e),
                }
                cx.notify();
            });
        });
        Self {
            root,
            git: None,
            dirs: None,
            state: Remote::Loading,
            busy: None,
            plan: None,
            full: Rc::new(()),
            light: Rc::new(()),
            reading: None,
            again: false,
            stale: true,
            opening: Some(opening),
            ticking: None,
        }
    }

    fn wants_full(&self) -> bool {
        Rc::strong_count(&self.full) > 1
    }

    fn looked(&self) -> bool {
        self.wants_full() || Rc::strong_count(&self.light) > 1
    }

    /// Reads the whole state (a git panel) while the handle lives.
    pub fn look(&mut self, cx: &mut Context<Self>) -> Looking {
        let upgrade = !self.wants_full() && !self.snapshot().is_some_and(|s| s.full);
        let handle = Looking(self.full.clone());
        if self.stale || upgrade {
            self.refresh(cx);
        }
        handle
    }

    /// Reads the branches only (a service or app view) while the handle lives.
    pub fn glance(&mut self, cx: &mut Context<Self>) -> Looking {
        let first = !self.looked();
        let handle = Looking(self.light.clone());
        if first && self.stale {
            self.refresh(cx);
        }
        handle
    }

    /// Git's files changed (the watcher): read again if someone looks, else later.
    pub fn changed(&mut self, cx: &mut Context<Self>) {
        // Its own operation reads once git is done.
        if self.busy.is_some() {
            return;
        }
        if self.looked() {
            self.refresh(cx);
        } else {
            self.stale = true;
        }
    }

    /// Reads again (one read at a time): the full state when a panel looks, else the
    /// branches.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(git) = self.git.clone() else { return };
        let fetch_head = self.dirs.as_ref().map(|d| d.git_dir.join("FETCH_HEAD"));
        if self.reading.is_some() {
            self.again = true;
            return;
        }
        self.stale = false;
        let full = self.wants_full();
        self.reading = Some(cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    let (status, blocked) = if full {
                        (git.status()?, git.writes_blocked()?)
                    } else {
                        (Status { head: git.head()?, upstream: None, changes: Vec::new(), doing: None }, None)
                    };
                    let branches = git.branches()?;
                    let fetched = fetch_head
                        .and_then(|f| std::fs::metadata(f).ok()?.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    Ok::<_, String>(Snapshot { status, branches, blocked, fetched, full })
                })
                .await;
            let _ = this.update(cx, |r, cx| {
                r.reading = None;
                let state = match read {
                    Ok(s) => Remote::Ready(s),
                    Err(e) => Remote::Error(e),
                };
                if r.state != state {
                    r.state = state;
                    cx.notify();
                }
                if std::mem::take(&mut r.again) || (r.wants_full() && !r.snapshot().is_some_and(|s| s.full)) {
                    r.refresh(cx);
                }
            });
        }));
    }

    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.state.ready()
    }

    /// The full state, when read (a git panel's).
    pub fn full(&self) -> Option<&Snapshot> {
        self.snapshot().filter(|s| s.full)
    }

    /// The core's handle, for reads a view makes itself (a fresh status before asking).
    pub fn git(&self) -> Option<Git> {
        self.git.clone()
    }

    /// The operation running, with its elapsed time ("switching to x… 12 s").
    pub fn busy_text(&self) -> Option<String> {
        self.busy.as_ref().map(|(what, since)| {
            let s = since.elapsed().as_secs();
            if s < 2 { what.clone() } else { format!("{what} {s} s") }
        })
    }

    /// Why `op` can't run now, if it can't: another operation running, or one under way
    /// in the repository (only an abort then).
    pub fn refusal(&self, op: &Op) -> Option<String> {
        if let Some((busy, _)) = &self.busy {
            return Some(format!("{busy} first"));
        }
        if self.git.is_none() {
            return Some("the repository isn't read yet".into());
        }
        let Some(s) = self.full() else { return Some("the repository isn't read yet".into()) };
        match (op, s.blocked.clone()) {
            (Op::Abort(_), None) => Some("nothing to abort".into()),
            (Op::Abort(_), Some(_)) => None,
            (_, Some(why)) => Some(why),
            (_, None) => None,
        }
    }

    fn start(&mut self, doing: String, cx: &mut Context<Self>) {
        self.busy = Some((doing, Instant::now()));
        self.ticking = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        }));
        cx.notify();
    }

    fn finish(&mut self) {
        self.busy = None;
        self.ticking = None;
    }

    /// Runs `op` (`what`: it in words, `doing`: while it runs); a dangerous one is
    /// refused ([`Repo::run_confirmed`]).
    pub fn run(&mut self, op: Op, what: String, doing: String, cx: &mut Context<Self>) -> Result<(), String> {
        if op.dangerous() {
            return Err("it needs confirming first".into());
        }
        self.send(op, what, doing, cx)
    }

    /// Runs `op` as [`Repo::run`] does, a dangerous one too: the user confirmed it.
    pub fn run_confirmed(&mut self, op: Op, _: Confirmed, what: String, doing: String, cx: &mut Context<Self>) -> Result<(), String> {
        self.send(op, what, doing, cx)
    }

    /// The task holds the repository until git exits.
    fn send(&mut self, op: Op, what: String, doing: String, cx: &mut Context<Self>) -> Result<(), String> {
        if let Some(why) = self.refusal(&op) {
            return Err(why);
        }
        let git = self.git.clone().ok_or("the repository isn't read yet")?;
        self.start(doing, cx);
        let me = cx.entity();
        cx.spawn(async move |_, cx| {
            #[allow(clippy::disallowed_methods)] // Dangerous ops reach it only through run_confirmed.
            let result = cx.background_executor().spawn(async move { git.run(&op) }).await;
            me.update(cx, |r, cx| {
                r.finish();
                if let Err(e) = &result {
                    devdeck_ui::toasts(cx).update(cx, |t, cx| t.push(devdeck_ui::ToastKind::Warn, format!("{what}: {e}"), vec![], cx));
                }
                r.refresh(cx);
                cx.notify();
            });
        })
        .detach();
        Ok(())
    }

    /// Fetches the push remote and decides what a push would do ([`Repo::plan`] once
    /// known).
    pub fn plan_push(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if let Some(why) = self.refusal(&Op::Fetch) {
            return Err(why);
        }
        let git = self.git.clone().ok_or("the repository isn't read yet")?;
        self.start("checking the remote…".into(), cx);
        self.plan = None;
        let me = cx.entity();
        cx.spawn(async move |_, cx| {
            let plan = cx.background_executor().spawn(async move { git.push_plan() }).await;
            me.update(cx, |r, cx| {
                r.finish();
                r.plan = Some(plan);
                r.refresh(cx);
                cx.notify();
            });
        })
        .detach();
        Ok(())
    }

    /// The push plan just decided, taken by the view that asked.
    pub fn take_plan(&mut self) -> Option<Result<PushPlan, String>> {
        self.plan.take()
    }

    /// The branch checked out, as last read.
    pub fn head(&self) -> Option<&Head> {
        self.snapshot().map(|s| &s.status.head)
    }

    /// Whether `path` is one of git's files this repository's views care about.
    pub fn concerns(&self, path: &Path) -> bool {
        let Some(d) = &self.dirs else { return false };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if name.ends_with(".lock") {
            return false;
        }
        let in_dir = |dir: &Path| path.parent() == Some(dir);
        let top = [
            "HEAD", "index", "FETCH_HEAD", "ORIG_HEAD", "MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "BISECT_LOG", "packed-refs", "rebase-merge",
            "rebase-apply", "sequencer",
        ];
        (in_dir(&d.git_dir) || in_dir(&d.common_dir)) && top.contains(&name)
            || path.starts_with(d.common_dir.join("refs"))
            || path.starts_with(d.common_dir.join("reftable"))
    }

    /// The folders to watch for it: the git dirs (not recursively) and the refs (recursively).
    pub fn watch(&self) -> Vec<(PathBuf, bool)> {
        let Some(d) = &self.dirs else { return Vec::new() };
        let mut out = vec![(d.git_dir.clone(), false)];
        if d.common_dir != d.git_dir {
            out.push((d.common_dir.clone(), false));
        }
        out.push((d.common_dir.join("refs"), true));
        let reftable = d.common_dir.join("reftable");
        if reftable.is_dir() {
            out.push((reftable, true));
        }
        out
    }

    /// Whether it's still being opened (tests).
    /// A read is in flight.
    pub fn reading(&self) -> bool {
        self.reading.is_some()
    }

    pub fn opening(&self) -> bool {
        self.opening.is_some()
    }
}

/// The root of the repository holding `dir`: the nearest folder with a `.git` (a folder,
/// or a file for a worktree or submodule), `dir` included, no higher than `ceiling` (the
/// workspace folder; `None`: no bound).
pub fn root_of(dir: &Path, ceiling: Option<&Path>) -> Option<PathBuf> {
    dir.ancestors().take_while(|d| ceiling.is_none_or(|c| d.starts_with(c))).find(|d| d.join(".git").exists()).map(Path::to_path_buf)
}
