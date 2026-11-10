//! Git and ports, read (R13.1): what the git panel and the port views show, for the
//! workspace's apps only. A read holds what views hold while they're on screen (a
//! repository looked at, a port scan with its ports watched) until it has its answer, and
//! counts among its caller's waits; nothing goes on reading for it after, but the read in
//! flight (git's own limit) and at most one read the repository had already queued.
//!
//! Repositories are those of the folders `file.read` allows (no hidden app, never `/` or
//! the home folder), never above the app's workspace folder (an app outside every folder:
//! its own folder at most); finding them reads the disk, in the background. The demo has
//! none. Paths whose name says they may hold secrets are counted, never listed.

use super::dispatch::CallCx;
use super::handlers::{secret, workspace};
use crate::runner::RunKey;
use crate::services::Services;
use devdeck_api::types::{Branches, BranchesParams, GitBranch, GitChange, GitHead, GitStatus, GitUpstream, InWorkspace, PortEntry, PortList, PortParams, RepoInfo, RepoParams, Repos};
use devdeck_api::{ApiError, ErrorKind};
use devdeck_core::git::Head;
use gpui::{App, AppContext, AsyncApp, Entity, Task};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

/// How long a repository read is waited for, and a port scan.
pub const GIT_WITHIN: Duration = Duration::from_secs(10);
pub const PORTS_WITHIN: Duration = Duration::from_secs(5);
/// How often a read looks whether its answer is there.
const POLL: Duration = Duration::from_millis(100);
const MAX_CHANGES: usize = 500;
const DEFAULT_BRANCHES: usize = 200;
const MAX_BRANCHES: usize = 1000;

/// What finding the roots needs, taken on the UI thread (no disk).
struct Apps {
    apps: Vec<(String, PathBuf)>,
    folders: Vec<PathBuf>,
}

fn apps_of(s: &Services) -> Option<Apps> {
    let config = s.config.as_ref()?;
    let hidden = |id: &str| config.apps.get(id).is_some_and(|o| o.hidden);
    let apps = s.catalog.apps.iter().filter(|a| !a.id.is_empty() && !hidden(&a.id)).map(|a| (a.id.clone(), a.dir.clone())).collect();
    Some(Apps { apps, folders: config.folders.clone() })
}

/// The repositories of `apps`, by root, with their apps (reads the disk).
fn roots(apps: Option<Apps>) -> BTreeMap<PathBuf, Vec<String>> {
    let Some(apps) = apps else { return BTreeMap::new() };
    let home = std::env::var_os("HOME").map(PathBuf::from).and_then(|h| h.canonicalize().ok());
    let folders: Vec<PathBuf> = apps.folders.iter().filter_map(|f| f.canonicalize().ok()).collect();
    let mut out: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for (id, dir) in apps.apps {
        let Ok(dir) = dir.canonicalize() else { continue };
        if dir.parent().is_none() || home.as_ref().is_some_and(|h| h.starts_with(&dir)) {
            continue;
        }
        let ceiling = folders.iter().find(|f| dir.starts_with(f)).cloned().unwrap_or_else(|| dir.clone());
        // A workspace folder may be the home folder: its repository (dotfiles) isn't an app's.
        let Some(root) = crate::repo::root_of(&dir, Some(&ceiling)) else { continue };
        if root.parent().is_none() || home.as_ref().is_some_and(|h| h.starts_with(&root)) {
            continue;
        }
        out.entry(root).or_default().push(id);
    }
    out
}

pub fn git_repos(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Task<Result<Repos, ApiError>>, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let apps = apps_of(w.services.read(app));
    Ok(app.background_executor().spawn(async move {
        let repos = roots(apps).into_iter().map(|(root, apps)| RepoInfo { root: root.display().to_string().into(), apps }).collect();
        Ok(Repos { repos })
    }))
}

/// The repository at `root`, one of the workspace's: found in the background, opened on
/// the UI thread. Unknown and another workspace's are the same `not_found`.
async fn repo_at(services: &Entity<Services>, apps: Option<Apps>, root: String, cx: &mut AsyncApp) -> Result<Entity<crate::repo::Repo>, ApiError> {
    let missing = || ApiError::not_found(format!("no repository {root} in this workspace"));
    let wanted = PathBuf::from(&root);
    let known = cx.background_spawn(async move { roots(apps) }).await;
    let root = known.into_keys().find(|r| *r == wanted).ok_or_else(missing)?;
    cx.update(|app| services.update(app, |s, cx| s.repo(&root, cx))).ok_or_else(missing)
}

/// Polls `ready` every [`POLL`] until it gives an answer, `within` at most.
async fn until<T>(cx: &mut AsyncApp, within: Duration, what: &str, mut ready: impl FnMut(&mut App) -> Option<Result<T, ApiError>>) -> Result<T, ApiError> {
    let deadline = cx.background_executor().now() + within;
    loop {
        if let Some(r) = cx.update(&mut ready) {
            return r;
        }
        if cx.background_executor().now() >= deadline {
            return Err(ApiError::unavailable(format!("{what} took longer than {} s", within.as_secs())));
        }
        cx.background_executor().timer(POLL).await;
    }
}

/// Why a repository can't be read now, if it can't (a read in flight may yet succeed).
fn unreadable(r: &crate::repo::Repo) -> Option<ApiError> {
    if r.reading() {
        return None;
    }
    if let devdeck_core::cluster::Remote::Error(e) = &r.state {
        return Some(ApiError::new(ErrorKind::Refused, e.clone()));
    }
    (r.git().is_none() && !r.opening()).then(|| ApiError::new(ErrorKind::Refused, "git can't open it"))
}

fn lower(d: impl std::fmt::Debug) -> String {
    format!("{d:?}").to_lowercase()
}

pub fn git_status(cx: &CallCx, p: RepoParams, app: &mut App) -> Result<Task<Result<GitStatus, ApiError>>, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let held = super::waits::hold(cx.who, app)?;
    let apps = apps_of(w.services.read(app));
    let services = w.services.clone();
    Ok(app.spawn(async move |cx| {
        let _held = held;
        let repo = repo_at(&services, apps, p.root, cx).await?;
        // The whole state, as a git panel reads it, while this waits.
        let _looking = cx.update(|app| repo.update(app, |r, cx| r.look(cx)));
        until(cx, GIT_WITHIN, "the repository's read", |app| {
            let r = repo.read(app);
            if let Some(e) = unreadable(r) {
                return Some(Err(e));
            }
            let s = r.full().filter(|_| !r.reading())?;
            Some(Ok(status(&s.status)))
        })
        .await
    }))
}

fn status(s: &devdeck_core::git::Status) -> GitStatus {
    let (kind, name) = match &s.head {
        Head::Branch(b) => ("branch", b),
        Head::Detached(c) => ("detached", c),
        Head::Unborn(b) => ("unborn", b),
    };
    let (shown, hidden): (Vec<_>, Vec<_>) = s.changes.iter().partition(|c| !secret(std::path::Path::new(&c.path)) && !c.from.as_deref().is_some_and(|f| secret(std::path::Path::new(f))));
    let changes: Vec<GitChange> = shown
        .iter()
        .take(MAX_CHANGES)
        .map(|c| GitChange { path: c.path.as_str().into(), from: c.from.as_deref().map(Into::into), kind: lower(c.kind), staged: c.staged })
        .collect();
    GitStatus {
        head: GitHead { kind: kind.into(), name: name.as_str().into() },
        upstream: s.upstream.as_ref().map(|u| GitUpstream { name: u.name.as_str().into(), ahead: u.ahead, behind: u.behind, gone: u.gone }),
        doing: s.doing.as_ref().map(lower),
        truncated: shown.len() > changes.len(),
        changes,
        hidden: hidden.len() as u32,
    }
}

pub fn git_branches(cx: &CallCx, p: BranchesParams, app: &mut App) -> Result<Task<Result<Branches, ApiError>>, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let held = super::waits::hold(cx.who, app)?;
    let apps = apps_of(w.services.read(app));
    let services = w.services.clone();
    let remote = p.remote.unwrap_or(false);
    let limit = p.limit.map_or(DEFAULT_BRANCHES, |n| n as usize).clamp(1, MAX_BRANCHES);
    Ok(app.spawn(async move |cx| {
        let _held = held;
        let repo = repo_at(&services, apps, p.root, cx).await?;
        // The branches only, as a service view reads them.
        let _looking = cx.update(|app| repo.update(app, |r, cx| r.glance(cx)));
        until(cx, GIT_WITHIN, "the repository's read", |app| {
            let r = repo.read(app);
            if let Some(e) = unreadable(r) {
                return Some(Err(e));
            }
            let s = r.snapshot().filter(|_| !r.reading())?;
            let all: Vec<GitBranch> = s
                .branches
                .iter()
                .filter(|b| remote || b.remote.is_none())
                .map(|b| GitBranch {
                    name: b.name.as_str().into(),
                    remote: b.remote.is_some(),
                    upstream: b.upstream.as_deref().map(Into::into),
                    commit: b.commit.clone(),
                    subject: b.subject.as_str().into(),
                    author: b.author.as_str().into(),
                })
                .collect();
            let truncated = all.len() > limit;
            Some(Ok(Branches { branches: all.into_iter().take(limit).collect(), truncated }))
        })
        .await
    }))
}

/// The ports a workspace's commands declare.
fn declared(s: &Services) -> BTreeSet<u16> {
    s.catalog.apps.iter().flat_map(|a| a.commands.iter().flat_map(|c| c.ports.iter().copied())).collect()
}

/// The workspace's ports now: declared or listened on by its runs, and who holds each
/// (another program by its name only).
fn ports_now(s: &Services, app: &App) -> Vec<PortEntry> {
    let infos = s.ports(app);
    let mut wanted = declared(s);
    wanted.extend(infos.iter().filter(|i| i.run.as_ref().is_some_and(|k| !k.is_terminal())).map(|i| i.port));
    wanted
        .into_iter()
        .map(|port| {
            let info = infos.iter().find(|i| i.port == port);
            let (app_id, cmd) = info.and_then(|i| i.run.as_ref()).filter(|k| !k.is_terminal()).map(|k: &RunKey| (k.app.clone(), k.cmd.clone())).unzip();
            let holder = info.map(holder_name);
            PortEntry { port, app: app_id, cmd, holder: holder.map(Into::into) }
        })
        .collect()
}

/// Who holds a port, as a caller may be told: a run's title; another program by its name
/// only (its arguments may hold tokens, its pid isn't the caller's to act on).
fn holder_name(i: &crate::services::PortInfo) -> String {
    match (&i.run, i.pid) {
        (Some(_), _) | (None, None) => i.holder.clone(),
        (None, Some(_)) => crate::port_scan::program(&i.command).unwrap_or("a process").to_string(),
    }
}

/// Answers `f` once the workspace's ports are scanned (a scan started for it when none
/// runs, `watch` watched with it), or at once when there's nothing to scan (the demo,
/// no runs and no watched port).
fn after_scan<T: 'static>(cx: &CallCx, services: Entity<Services>, watch: BTreeSet<u16>, app: &mut App, f: impl Fn(&Services, &App) -> Result<T, ApiError> + 'static) -> Result<Task<Result<T, ApiError>>, ApiError> {
    let held = super::waits::hold(cx.who, app)?;
    let ports = services.read(app).ports.clone();
    // A scan already under way doesn't watch the new ports yet: the one after it does.
    let in_flight = ports.read(app).scanning();
    let interest = ports.update(app, |p, cx| p.interest(watch, cx));
    let start = ports.read(app).scans() + u64::from(in_flight);
    Ok(app.spawn(async move |cx| {
        let _held = (held, interest);
        until(cx, PORTS_WITHIN, "the port scan", |app| {
            let p = ports.read(app);
            (!p.scanning() || p.scans() > start).then(|| f(services.read(app), app))
        })
        .await
    }))
}

pub fn port_list(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Task<Result<PortList, ApiError>>, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let watch = declared(w.services.read(app));
    after_scan(cx, w.services, watch, app, |s, app| Ok(PortList { ports: ports_now(s, app) }))
}

pub fn port_get(cx: &CallCx, p: PortParams, app: &mut App) -> Result<Task<Result<PortEntry, ApiError>>, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let port = p.port;
    after_scan(cx, w.services, BTreeSet::from([port]), app, move |s, app| {
        ports_now(s, app).into_iter().find(|e| e.port == port).ok_or_else(|| ApiError::not_found(format!("port {port} isn't one of this workspace's")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::PortInfo;

    #[test]
    fn another_programs_holder_is_its_name_only() {
        let foreign = PortInfo { port: 5432, holder: "node (pid 77)".into(), command: "/usr/bin/node server.js --token=s3cret".into(), run: None, pid: Some(77) };
        assert_eq!(holder_name(&foreign), "node");
        let ours = PortInfo { port: 3000, holder: "web · dev".into(), command: "pnpm dev".into(), run: Some(RunKey::new("web", "dev")), pid: None };
        assert_eq!(holder_name(&ours), "web · dev");
    }
}
