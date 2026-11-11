//! Shells for the caller, the current env, and devdeck's own data (R13.1, classes run and
//! organize): notes, layouts, pins and groups. What the user started, typed in or
//! protects asks the user (`approval_required`, the card in P4); what the caller made or
//! started follows its class. Caps keep what's saved small: a caller can't fill the
//! sidebar, the layouts or the notes.

use super::dispatch::CallCx;
use super::handlers::{file_roots, pulse_list, say, starter, terminal_get, workspace, workspace_get, WsRef};
use crate::runner::{RunKey, Starter};
use crate::sidebar::{Activity, Sidebar};
use devdeck_api::types::{
    Done, EnvParams, GroupCreateParams, GroupMoveParams, GroupParams, GroupPath, GroupPutParams, GroupRenameParams, InWorkspace, NameParams,
    NoteAppendParams, NoteCreateParams, NoteRenameParams, NoteRevision, NoteWriteParams, PinParams, Pulses, RenameParams, Terminal, TerminalOpenParams, TerminalParams,
    TerminalStopParams, Workspace,
};
use devdeck_api::{ApiError, ErrorKind};
use devdeck_core::layout::{Drop, Edge as CoreEdge, PaneId};
use devdeck_core::view_addr::ViewAddr;
use gpui::{App, Entity};
use std::time::Duration;

/// Shells one caller may have running, opened through the API.
pub const TERMINALS_PER_CALLER: usize = 4;
/// A note the user typed in this recently is theirs to change (D3).
pub const TYPED_WITHIN: Duration = Duration::from_secs(10);
/// Text one call writes at most, a note's name, notes in a workspace.
pub const WRITE_MAX: usize = 64 * 1024;
pub const NAME_MAX: usize = 100;
pub const NOTES_MAX: usize = 500;
pub const LAYOUTS_MAX: usize = 50;
pub const PINS_MAX: usize = 200;
pub const GROUPS_MAX: usize = 100;
/// Items put in groups, a list; an item's key as given.
pub const MEMBERS_MAX: usize = 500;
pub const KEY_MAX: usize = 200;

/// `node` must be one of `list`'s items or folders, its key of a sane length: a caller
/// can't grow what's saved with names of its own.
fn known(s: &Sidebar, list: Activity, node: &str, cx: &App) -> Result<(), ApiError> {
    if node.is_empty() || node.len() > KEY_MAX {
        return Err(ApiError::invalid(format!("key: 1 to {KEY_MAX} bytes")));
    }
    if !s.knows(list, node, cx) {
        return Err(ApiError::not_found(format!("{node} isn't in that list")));
    }
    Ok(())
}

fn asks(why: &str) -> ApiError {
    ApiError::new(ErrorKind::ApprovalRequired, format!("{why}: that needs the user's approval, which comes in a later version of devdeck"))
}

fn me(cx: &CallCx) -> Result<Starter, ApiError> {
    starter(cx.who).ok_or_else(|| ApiError::new(ErrorKind::PermissionDenied, "only a devdeck terminal does that"))
}

// ---- Shells ----

pub fn terminal_open(cx: &CallCx, p: TerminalOpenParams, app: &mut App) -> Result<Terminal, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let me = me(cx)?;
    let runner = w.services.read(app).runner.clone();
    // Every shell this terminal opened that's still known counts (stopped ones keep
    // their tab), and those waiting for the tools.
    if runner.read(app).shells_of(&me) >= TERMINALS_PER_CALLER {
        return Err(ApiError::new(ErrorKind::Busy, format!("{TERMINALS_PER_CALLER} shells of yours are open already (terminal.stop closes one)")));
    }
    super::present::room(&w, app)?;
    let services = w.services.read(app);
    if services.protected_env() {
        return Err(asks("the current env is protected"));
    }
    let env = services.env.clone();
    let dir = match &p.dir {
        Some(d) => {
            let asked = std::path::Path::new(d).canonicalize().map_err(|e| ApiError::not_found(format!("{d}: {e}")))?;
            let roots = file_roots(services);
            let folders: Vec<_> = services.config.iter().flat_map(|c| c.folders.iter().filter_map(|f| f.canonicalize().ok())).collect();
            if !roots.iter().chain(&folders).any(|r| asked.starts_with(r)) || !asked.is_dir() {
                return Err(ApiError::new(ErrorKind::PermissionDenied, format!("{d}: not a folder of the workspace's")));
            }
            asked
        }
        None => {
            let Starter::Caller { n, .. } = &me else { unreachable!("a caller") };
            runner.read(app).terminals().into_iter().find(|(k, _)| k.terminal_number() == Some(*n)).map(|(_, d)| d).ok_or_else(|| ApiError::invalid("dir: needed"))?
        }
    };
    // Where its tab goes, checked before a shell starts.
    let l = w.deck.read(app).layout();
    let pane = match p.pane {
        Some(pane) => {
            let layout = p.pulse.as_deref().ok_or_else(|| ApiError::invalid("pulse: needed with pane"))?;
            if w.deck.read(app).book.current != layout {
                return Err(ApiError::new(ErrorKind::Conflict, "that pulse isn't shown: read ui.get again"));
            }
            l.pane(PaneId(pane)).map_err(|_| ApiError::not_found(format!("no pane {pane}")))?;
            PaneId(pane)
        }
        None => l.main().focused,
    };
    let n = w.deck.read(app).next_terminal(app);
    // The runner first: the tab, built as the user's, finds it asked for by the caller.
    runner.update(app, |r, cx| r.open_shell(n, &dir, env, me.clone(), cx));
    // Started, or waiting for the tools; one that couldn't start gets no tab.
    if let Some(run) = runner.read(app).run(&RunKey::terminal(n))
        && !run.state.is_running()
    {
        let why = run.error.clone().unwrap_or_else(|| "it ended at once".into());
        return Err(ApiError::new(ErrorKind::Refused, format!("the shell couldn't start: {why}")));
    }
    let view = ViewAddr::Terminal { n, dir: dir.display().to_string() }.to_ref();
    let drop = match p.edge {
        Some(e) => Drop::Edge(pane, edge(e)),
        None => Drop::Into(pane),
    };
    let keys = crate::window_view::keys(&w.deck, app).ok_or_else(|| ApiError::new(ErrorKind::Busy, "a window is being drawn: try again"))?;
    let placed = w.deck.update(app, |d, cx| d.apply_quietly(&keys, None, cx, |l| l.open_at(view, drop)));
    if let Err(e) = placed {
        // Without its tab it would be forgotten: it doesn't stay (nor wait to start).
        runner.update(app, |r, cx| r.drop_shell(&RunKey::terminal(n), cx));
        return Err(match e {
            crate::deck::Quiet::Keys => asks("its tab would move what the user is typing in"),
            crate::deck::Quiet::Refused(why) => ApiError::new(ErrorKind::Refused, why),
        });
    }
    say(cx, "opened", &format!("terminal {n}"), app);
    terminal_get(cx, TerminalParams { workspace: Some(w.id.clone()), n }, app)
}

fn edge(e: devdeck_api::types::Edge) -> CoreEdge {
    use devdeck_api::types::Edge as E;
    match e {
        E::Left => CoreEdge::Left,
        E::Right => CoreEdge::Right,
        E::Top => CoreEdge::Top,
        E::Bottom => CoreEdge::Bottom,
    }
}

pub fn terminal_stop(cx: &CallCx, p: TerminalStopParams, app: &mut App) -> Result<Terminal, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let me = me(cx)?;
    let runner = w.services.read(app).runner.clone();
    let key = RunKey::terminal(p.n);
    let r = runner.read(app);
    if r.terminal_uid(&key) != Some(p.uid.as_str()) {
        return Err(ApiError::not_found(format!("no terminal {} with uid {}", p.n, p.uid)));
    }
    let run = r.run(&key).ok_or_else(|| ApiError::not_found(format!("terminal {} isn't running", p.n)))?;
    if run.starter != me {
        return Err(asks(&format!("terminal {} is the user's", p.n)));
    }
    if let Some(program) = r.terminal_program(&key) {
        return Err(asks(&format!("terminal {} runs {program}", p.n)));
    }
    runner.update(app, |r, cx| r.stop(&key, cx));
    // Its tab goes too (the caller made it), unless the user types in it now.
    let tab = w.deck.read(app).layout().views().into_iter().find(|r| matches!(ViewAddr::parse(r), ViewAddr::Terminal { n, .. } if n == p.n));
    if let Some(tab) = tab
        && let Some(keys) = crate::window_view::keys(&w.deck, app)
    {
        let _ = w.deck.update(app, |d, cx| d.apply_quietly(&keys, None, cx, |l| l.close_view(&tab)));
    }
    say(cx, "stopped", &format!("terminal {}", p.n), app);
    terminal_get(cx, TerminalParams { workspace: Some(w.id.clone()), n: p.n }, app)
}

// ---- The current env ----

fn sidebar(w: &WsRef, app: &App) -> Result<Entity<Sidebar>, ApiError> {
    let shell = crate::shell::try_shell(app).ok_or_else(|| ApiError::unavailable("no shell"))?;
    shell.read(app).workspaces.iter().find(|x| x.id == w.id).map(|x| x.sidebar.clone()).ok_or_else(|| ApiError::not_found("no sidebar"))
}

pub fn env_set_current(cx: &CallCx, p: EnvParams, app: &mut App) -> Result<Workspace, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let protected = w.services.read(app).catalog.env(&p.name).ok_or_else(|| ApiError::not_found(format!("no env {}", p.name)))?.protected;
    if protected {
        return Err(asks(&format!("{} is protected", p.name)));
    }
    sidebar(&w, app)?.update(app, |s, cx| s.set_env(&p.name, cx)).map_err(ApiError::not_found)?;
    say(cx, "made the current env", &p.name, app);
    workspace_get(cx, InWorkspace { workspace: Some(w.id.clone()) }, app)
}

// ---- Notes ----

fn name(n: &str) -> Result<String, ApiError> {
    let n = n.trim();
    if n.is_empty() || n.chars().count() > NAME_MAX || n.chars().any(char::is_control) {
        return Err(ApiError::invalid(format!("name: one line of 1 to {NAME_MAX} characters")));
    }
    Ok(n.to_string())
}

fn revision(w: &WsRef, id: &str, app: &App) -> NoteRevision {
    NoteRevision { id: id.to_string(), revision: w.services.read(app).notes.read(app).revision(id) }
}

pub fn note_create(cx: &CallCx, p: NoteCreateParams, app: &mut App) -> Result<NoteRevision, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let n = name(&p.name)?;
    let s = w.services.read(app);
    let app_id = p.app.unwrap_or_default();
    let hidden = s.config.as_ref().is_some_and(|c| c.apps.get(&app_id).is_some_and(|o| o.hidden));
    if !app_id.is_empty() && (s.catalog.app(&app_id).is_none() || hidden) {
        return Err(ApiError::not_found(format!("no app {app_id}")));
    }
    let notes = s.notes.clone();
    if notes.read(app).all().len() >= NOTES_MAX {
        return Err(ApiError::new(ErrorKind::Refused, format!("{NOTES_MAX} notes at most in a workspace")));
    }
    let id = notes.update(app, |x, cx| x.create(&app_id, &n, cx)).ok_or_else(|| ApiError::new(ErrorKind::Refused, "the note wasn't made"))?;
    say(cx, "made the note", &n, app);
    Ok(revision(&w, &id, app))
}

/// Refused unless the caller may change note `id` now: it exists, it's readable, and the
/// user hasn't typed in it within [`TYPED_WITHIN`].
fn writable(w: &WsRef, id: &str, app: &mut App) -> Result<Entity<crate::notes::Notes>, ApiError> {
    let notes = w.services.read(app).notes.clone();
    if notes.read(app).meta(id).is_none() {
        return Err(ApiError::not_found(format!("no note {id}")));
    }
    let now = app.background_executor().now();
    if notes.read(app).typed_at(id).is_some_and(|t| now.duration_since(t) < TYPED_WITHIN) {
        return Err(asks("the user is writing in this note"));
    }
    Ok(notes)
}

fn write(cx: &CallCx, w: &WsRef, id: &str, text: String, base: Option<u64>, app: &mut App) -> Result<NoteRevision, ApiError> {
    let notes = writable(w, id, app)?;
    if let Some(base) = base
        && notes.read(app).revision(id) != base
    {
        return Err(ApiError::new(ErrorKind::Conflict, "the note changed since that revision: read it again"));
    }
    if text.len() as u64 > crate::notes::MAX_NOTE {
        return Err(ApiError::new(ErrorKind::Refused, format!("a note holds {} bytes at most", crate::notes::MAX_NOTE)));
    }
    notes.update(app, |n, cx| n.write(id, text, cx)).map_err(|e| ApiError::new(ErrorKind::Refused, e))?;
    let title = notes.read(app).meta(id).map(|m| m.name.clone()).unwrap_or_default();
    say(cx, "wrote in the note", &title, app);
    Ok(revision(w, id, app))
}

pub fn note_write(cx: &CallCx, p: NoteWriteParams, app: &mut App) -> Result<NoteRevision, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    if p.text.len() > WRITE_MAX {
        return Err(ApiError::invalid(format!("text: {WRITE_MAX} bytes at most a call")));
    }
    write(cx, &w, &p.id, p.text, Some(p.base_revision), app)
}

pub fn note_append(cx: &CallCx, p: NoteAppendParams, app: &mut App) -> Result<NoteRevision, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    if p.text.len() > WRITE_MAX {
        return Err(ApiError::invalid(format!("text: {WRITE_MAX} bytes at most a call")));
    }
    let notes = writable(&w, &p.id, app)?;
    let text = notes.update(app, |n, _| n.text(&p.id)) + &p.text;
    write(cx, &w, &p.id, text, p.base_revision, app)
}

pub fn note_rename(cx: &CallCx, p: NoteRenameParams, app: &mut App) -> Result<NoteRevision, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let n = name(&p.name)?;
    let notes = writable(&w, &p.id, app)?;
    notes.update(app, |x, cx| x.rename(&p.id, &n, cx));
    if notes.read(app).meta(&p.id).is_none_or(|m| m.name != n) {
        return Err(ApiError::new(ErrorKind::Refused, "the note wasn't renamed"));
    }
    say(cx, "renamed a note to", &n, app);
    Ok(revision(&w, &p.id, app))
}

// ---- Pulses ----

pub fn pulse_save_as(cx: &CallCx, p: NameParams, app: &mut App) -> Result<Pulses, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let n = name(&p.name)?;
    if w.deck.read(app).book.entries().len() >= LAYOUTS_MAX {
        return Err(ApiError::new(ErrorKind::Refused, format!("{LAYOUTS_MAX} pulses at most")));
    }
    w.deck.update(app, |d, cx| d.save_layout_as(&n, cx)).map_err(|e| ApiError::new(ErrorKind::Refused, e.to_string()))?;
    say(cx, "saved the pulse as", &n, app);
    pulse_list(cx, InWorkspace { workspace: Some(w.id.clone()) }, app)
}

pub fn pulse_rename(cx: &CallCx, p: RenameParams, app: &mut App) -> Result<Pulses, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let n = name(&p.name)?;
    w.deck.update(app, |d, cx| d.rename_layout(&p.id, &n, cx)).map_err(|e| ApiError::new(ErrorKind::Refused, e.to_string()))?;
    say(cx, "renamed a pulse to", &n, app);
    pulse_list(cx, InWorkspace { workspace: Some(w.id.clone()) }, app)
}

// ---- Pins and groups ----

/// A sidebar list by its id (`apps`, `cluster`…), and the workspace's sidebar.
fn list(cx: &CallCx, ws: Option<&str>, id: &str, app: &App) -> Result<(Entity<Sidebar>, Activity), ApiError> {
    let w = workspace(cx, ws, app)?;
    let list = Activity::from_id(id).ok_or_else(|| ApiError::not_found(format!("no list {id}")))?;
    Ok((sidebar(&w, app)?, list))
}

fn groups_of(s: &Sidebar, list: Activity) -> Vec<String> {
    s.state.groups.get(list.list_id()).map(|g| g.paths()).unwrap_or_default()
}

fn group_exists(s: &Sidebar, list: Activity, path: &str) -> Result<(), ApiError> {
    if !groups_of(s, list).iter().any(|g| g == path) {
        return Err(ApiError::not_found(format!("no group {path}")));
    }
    Ok(())
}

pub fn sidebar_pin(cx: &CallCx, p: PinParams, app: &mut App) -> Result<Done, ApiError> {
    let (s, list) = list(cx, p.workspace.as_deref(), &p.list, app)?;
    if list == Activity::Terminals {
        return Err(ApiError::new(ErrorKind::Refused, "terminals aren't pinned"));
    }
    if p.pinned {
        known(s.read(app), list, &p.key, app)?;
    }
    let pins = s.read(app).state.pins.get(list.list_id()).map_or(0, |x| x.len());
    if p.pinned && pins >= PINS_MAX {
        return Err(ApiError::new(ErrorKind::Refused, format!("{PINS_MAX} pins a list at most")));
    }
    s.update(app, |s, cx| s.set_pinned(list, &p.key, p.pinned, cx)).map_err(|e| ApiError::new(ErrorKind::Refused, e))?;
    Ok(Done {})
}

pub fn group_create(cx: &CallCx, p: GroupCreateParams, app: &mut App) -> Result<GroupPath, ApiError> {
    let (s, list) = list(cx, p.workspace.as_deref(), &p.list, app)?;
    let n = name(&p.name)?;
    if groups_of(s.read(app), list).len() >= GROUPS_MAX {
        return Err(ApiError::new(ErrorKind::Refused, format!("{GROUPS_MAX} groups a list at most")));
    }
    if let Some(parent) = &p.parent {
        group_exists(s.read(app), list, parent)?;
    }
    let path = s.update(app, |s, cx| s.create_group(list, p.parent.as_deref(), &n, cx)).map_err(|e| ApiError::new(ErrorKind::Refused, e))?;
    Ok(GroupPath { path: path.into() })
}

pub fn group_rename(cx: &CallCx, p: GroupRenameParams, app: &mut App) -> Result<GroupPath, ApiError> {
    let (s, list) = list(cx, p.workspace.as_deref(), &p.list, app)?;
    let n = name(&p.name)?;
    group_exists(s.read(app), list, &p.path)?;
    let path = s.update(app, |s, cx| s.rename_group(list, &p.path, &n, cx)).map_err(|e| ApiError::new(ErrorKind::Refused, e))?;
    Ok(GroupPath { path: path.into() })
}

pub fn group_move(cx: &CallCx, p: GroupMoveParams, app: &mut App) -> Result<GroupPath, ApiError> {
    let (s, list) = list(cx, p.workspace.as_deref(), &p.list, app)?;
    group_exists(s.read(app), list, &p.path)?;
    if let Some(parent) = &p.parent {
        group_exists(s.read(app), list, parent)?;
    }
    let path = s.update(app, |s, cx| s.move_group(list, &p.path, p.parent.as_deref(), cx)).map_err(|e| ApiError::new(ErrorKind::Refused, e))?;
    Ok(GroupPath { path: path.into() })
}

pub fn group_put(cx: &CallCx, p: GroupPutParams, app: &mut App) -> Result<Done, ApiError> {
    let (s, list) = list(cx, p.workspace.as_deref(), &p.list, app)?;
    if !Sidebar::groups_allowed_in(list) {
        return Err(ApiError::invalid(format!("{} aren't grouped", p.list)));
    }
    if let Some(g) = &p.group {
        group_exists(s.read(app), list, g)?;
    }
    known(s.read(app), list, &p.node, app)?;
    let members = s.read(app).state.groups.get(list.list_id()).map_or(0, |g| g.members.len());
    if members >= MEMBERS_MAX {
        return Err(ApiError::new(ErrorKind::Refused, format!("{MEMBERS_MAX} items in groups a list at most")));
    }
    s.update(app, |s, cx| s.put(list, &p.node, p.group.as_deref(), cx));
    Ok(Done {})
}

pub fn group_delete(cx: &CallCx, p: GroupParams, app: &mut App) -> Result<Done, ApiError> {
    let (s, list) = list(cx, p.workspace.as_deref(), &p.list, app)?;
    group_exists(s.read(app), list, &p.path)?;
    s.update(app, |s, cx| s.delete_group(list, &p.path, cx));
    Ok(Done {})
}
