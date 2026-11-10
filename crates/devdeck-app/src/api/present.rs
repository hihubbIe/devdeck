//! What the user sees, changed by the control API (R13.1, class present): views, panes,
//! windows, layouts, settings, toasts. Every layout change goes through
//! [`Deck::apply_quietly`](crate::deck::Deck::apply_quietly): no window's focused pane
//! changes and nothing the user types in is hidden or moved; when a change would, it asks
//! the user (`approval_required`: the card comes in P4). Panes, windows and splits are
//! named with the live layout's id (`ui.get`): another layout's ids are a `conflict`.
//! Closing the tab of a terminal that isn't running forgets it with its output: that asks.

use super::dispatch::CallCx;
use super::handlers::{devdecks_own, file_roots, readable, say, ui_get, workspace, WsRef};
use crate::deck::Quiet;
use crate::views::file_view::FileView;
use crate::views::note_view::NoteView;
use devdeck_api::types::{
    Done, Edge, FileOpenParams, IdParams, InWorkspace, MoveTabParams, NoParams, NoticeKind, Notified, NotifyParams, PaneCollapseParams, PaneMaximizeParams, PaneParams,
    PaneResizeParams, PaneSplitParams, RenderedParams, Settings, SettingsSetParams, ToastItem, Toasts, Ui, View, ViewOpenParams, ViewParams, WindowAttachParams,
    WindowParams,
};
use devdeck_api::{ApiError, ErrorKind};
use devdeck_core::layout::{Drop, Edge as CoreEdge, Layout, PaneId, SplitId, ViewRef, WindowId, MAIN_WINDOW};
use devdeck_core::view_addr::ViewAddr;
use gpui::{App, AppContext, Global, Task};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A toast from `ui.notify`: one line, this long at most; one per caller this often.
pub const NOTIFY_CHARS: usize = 200;
pub const NOTIFY_EVERY: Duration = Duration::from_secs(2);
/// A workspace is rescanned this often at most.
pub const RESCAN_EVERY: Duration = Duration::from_secs(5);

fn asks(why: &str) -> ApiError {
    ApiError::new(ErrorKind::ApprovalRequired, format!("{why}: that needs the user's approval, which comes in a later version of devdeck"))
}

/// devdeck's address of a view the API names.
fn addr(v: &View) -> Result<ViewAddr, ApiError> {
    Ok(match v.clone() {
        View::App { app } => ViewAddr::App { app },
        View::Run { app, cmd } => ViewAddr::Run { app, cmd },
        View::Svc { env, name } => ViewAddr::Svc { env, name },
        View::Logs { env, name } => ViewAddr::Logs { env, name },
        View::Pod { env, pod } => ViewAddr::Pod { env, pod },
        View::Matrix => ViewAddr::Matrix,
        View::Port { port } => ViewAddr::Port { port },
        View::Problems => ViewAddr::Problems,
        View::Welcome => ViewAddr::Welcome,
        View::Terminal { n, dir } => ViewAddr::Terminal { n, dir: dir.0 },
        View::Git { root } => ViewAddr::Git { root: root.0 },
        View::File { path } => ViewAddr::File { path: path.0 },
        View::Note { id } => ViewAddr::Note { id },
        View::Other { .. } | View::Unknown => return Err(ApiError::invalid("view: not one devdeck can name")),
    })
}

fn edge(e: Edge) -> CoreEdge {
    match e {
        Edge::Left => CoreEdge::Left,
        Edge::Right => CoreEdge::Right,
        Edge::Top => CoreEdge::Top,
        Edge::Bottom => CoreEdge::Bottom,
    }
}

/// `layout` must be the live one: ids repeat across layouts.
fn live(w: &WsRef, layout: &str, app: &App) -> Result<(), ApiError> {
    let current = &w.deck.read(app).book.current;
    if current != layout {
        return Err(ApiError::new(ErrorKind::Conflict, format!("layout {layout} isn't shown (layout {current} is): read ui.get again")));
    }
    Ok(())
}

/// Where a view goes: into `pane` or beside it on `edge`; the main window's focused pane
/// when none is named.
fn place(w: &WsRef, layout: Option<&str>, pane: Option<u64>, e: Option<Edge>, app: &App) -> Result<Drop, ApiError> {
    let l = w.deck.read(app).layout();
    let p = match pane {
        Some(p) => {
            live(w, layout.ok_or_else(|| ApiError::invalid("layout: needed with pane"))?, app)?;
            let p = PaneId(p);
            l.pane(p).map_err(|_| ApiError::not_found(format!("no pane {}", p.0)))?;
            p
        }
        None => l.main().focused,
    };
    Ok(match e {
        Some(e) => Drop::Edge(p, edge(e)),
        None => Drop::Into(p),
    })
}

/// A layout change the user didn't make: refused, it says why; taking the user's keys,
/// it asks.
fn quietly<T>(w: &WsRef, target: Option<PaneId>, app: &mut App, op: impl FnOnce(&mut Layout) -> devdeck_core::layout::Result<T>) -> Result<T, ApiError> {
    let keys = crate::window_view::keys(&w.deck, app).ok_or_else(|| ApiError::new(ErrorKind::Busy, "a window is being drawn: try again"))?;
    w.deck.update(app, |d, cx| d.apply_quietly(&keys, target, cx, op)).map_err(|e| match e {
        Quiet::Refused(why) => ApiError::new(ErrorKind::Refused, why),
        Quiet::Keys => asks("it would move or hide what the user is typing in"),
    })
}

/// Refused when closing `refs` forgets a terminal that isn't running (its output goes).
fn keeps_terminals(w: &WsRef, refs: &[ViewRef], app: &App) -> Result<(), ApiError> {
    let runner = w.services.read(app).runner.read(app);
    for r in refs {
        if let ViewAddr::Terminal { n, .. } = ViewAddr::parse(r)
            && !runner.state(&crate::runner::RunKey::terminal(n)).is_some_and(|s| s.is_running())
        {
            return Err(asks(&format!("terminal {n} isn't running: closing its tab forgets it and its output")));
        }
    }
    Ok(())
}

fn ui(cx: &CallCx, w: &WsRef, app: &mut App) -> Result<Ui, ApiError> {
    ui_get(cx, InWorkspace { workspace: Some(w.id.clone()) }, app)
}

/// Whether the workspace has the view's target (a junk tab would be saved with the layout).
fn exists(w: &WsRef, a: &ViewAddr, app: &App) -> Result<(), ApiError> {
    let s = w.services.read(app);
    let hidden = |id: &str| s.config.as_ref().is_some_and(|c| c.apps.get(id).is_some_and(|o| o.hidden));
    let ok = match a {
        ViewAddr::App { app: id } => s.catalog.app(id).is_some() && !hidden(id),
        ViewAddr::Run { app: id, cmd } => s.catalog.app(id).is_some_and(|x| x.command(cmd).is_some()) && !hidden(id),
        ViewAddr::Note { id } => s.notes.read(app).meta(id).is_some(),
        ViewAddr::Port { port } => port.parse::<u16>().is_ok_and(|p| super::gitports::ports_now(s, app).iter().any(|e| e.port == p)),
        _ => return Err(ApiError::invalid("view: an app, a run, a file, a note or a port (terminals: terminal.open)")),
    };
    if !ok {
        return Err(ApiError::not_found(format!("{} isn't one of this workspace's", a.to_ref().as_str())));
    }
    Ok(())
}

/// Opens `r` at `drop`; one already open moves only when a place was named, and shows
/// when asked.
fn open(w: &WsRef, r: ViewRef, drop: Drop, placed: bool, activate: bool, app: &mut App) -> Result<(), ApiError> {
    let existing = w.deck.read(app).layout().find_view(&r);
    match existing {
        Some((p, i)) if !placed => match activate {
            true => quietly(w, Some(p), app, |l| l.activate(p, i)),
            false => Ok(()),
        },
        _ => {
            let target = match drop {
                Drop::Into(p) if activate => Some(p),
                _ => None,
            };
            quietly(w, target, app, |l| l.open_at(r, drop))
        }
    }
}

pub fn view_open(cx: &CallCx, p: ViewOpenParams, app: &mut App) -> Result<Task<Result<Ui, ApiError>>, ApiError> {
    if let View::File { path } = &p.view {
        let f = FileOpenParams { workspace: p.workspace.clone(), path: path.0.clone(), line: None, layout: p.layout.clone(), pane: p.pane, edge: p.edge };
        return open_file(cx, f, p.activate.unwrap_or(false), app);
    }
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let a = addr(&p.view)?;
    exists(&w, &a, app)?;
    let drop = place(&w, p.layout.as_deref(), p.pane, p.edge, app)?;
    let title = a.to_ref();
    open(&w, a.to_ref(), drop, p.pane.is_some() || p.edge.is_some(), p.activate.unwrap_or(false), app)?;
    say(cx, "opened", &w.deck.read(app).title(&title, app), app);
    Ok(Task::ready(ui(cx, &w, app)))
}

/// `file.open` shows the file (its pane's active tab: in the pane holding the user's keys,
/// that asks), at a line when given.
pub fn file_open(cx: &CallCx, p: FileOpenParams, app: &mut App) -> Result<Task<Result<Ui, ApiError>>, ApiError> {
    open_file(cx, p, true, app)
}

/// A file of the workspace's apps' folders (as `file.read`: links resolved, devdeck's own
/// and secret-named files refused, the check in the background), opened by its real path.
fn open_file(cx: &CallCx, p: FileOpenParams, activate: bool, app: &mut App) -> Result<Task<Result<Ui, ApiError>>, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let roots = file_roots(w.services.read(app));
    let own = devdecks_own();
    let drop = place(&w, p.layout.as_deref(), p.pane, p.edge, app)?;
    let asked = std::path::PathBuf::from(&p.path);
    if !asked.is_absolute() {
        return Err(ApiError::invalid("path: absolute"));
    }
    let placed = p.pane.is_some() || p.edge.is_some();
    let (who, instance, state, conn) = (cx.who.clone(), cx.instance.to_string(), cx.state, cx.conn);
    Ok(app.spawn(async move |cx| {
        let checked = cx
            .background_spawn(async move {
                let path = asked.canonicalize().map_err(|e| ApiError::not_found(format!("{}: {e}", asked.display())))?;
                readable(&path, &roots, &own)?;
                if !path.is_file() {
                    return Err(ApiError::new(ErrorKind::Refused, format!("{}: not a file", path.display())));
                }
                Ok(path)
            })
            .await?;
        cx.update(|app| {
            let r = ViewAddr::File { path: checked.display().to_string() }.to_ref();
            open(&w, r.clone(), drop, placed, activate, app)?;
            if let Some(line) = p.line
                && let Some(v) = w.deck.read(app).view(&r).and_then(|v| v.downcast::<FileView>().ok())
            {
                v.update(app, |v, cx| v.go_to_line(line.max(1) as usize, cx));
            }
            let call = CallCx { who: &who, instance: &instance, state, conn };
            say(&call, "opened", &checked.display().to_string(), app);
            ui(&call, &w, app)
        })
    }))
}

/// An open view's pane and place.
fn find(w: &WsRef, v: &View, app: &App) -> Result<(ViewRef, PaneId, usize), ApiError> {
    let r = addr(v)?.to_ref();
    let (p, i) = w.deck.read(app).layout().find_view(&r).ok_or_else(|| ApiError::not_found(format!("{} isn't open", r.as_str())))?;
    Ok((r, p, i))
}

pub fn view_close(cx: &CallCx, p: ViewParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let (r, _, _) = find(&w, &p.view, app)?;
    keeps_terminals(&w, std::slice::from_ref(&r), app)?;
    let title = w.deck.read(app).title(&r, app);
    quietly(&w, None, app, |l| l.close_view(&r))?;
    say(cx, "closed", &title, app);
    ui(cx, &w, app)
}

pub fn view_focus(cx: &CallCx, p: ViewParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let (_, pane, i) = find(&w, &p.view, app)?;
    quietly(&w, Some(pane), app, |l| l.activate(pane, i))?;
    ui(cx, &w, app)
}

pub fn file_rendered(cx: &CallCx, p: RenderedParams, app: &mut App) -> Result<Done, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let (r, pane, _) = find(&w, &p.view, app)?;
    let keys = crate::window_view::keys(&w.deck, app).ok_or_else(|| ApiError::new(ErrorKind::Busy, "a window is being drawn: try again"))?;
    if keys.iter().any(|h| h.pane == pane && h.view.as_ref() == Some(&r)) {
        return Err(asks("the user is typing in it"));
    }
    let v = w.deck.read(app).view(&r).ok_or_else(|| ApiError::not_found("not built"))?;
    if let Ok(f) = v.clone().downcast::<FileView>() {
        if !f.update(app, |f, cx| f.set_rendered(p.rendered, cx)) {
            return Err(ApiError::invalid("not a Markdown file"));
        }
    } else if let Ok(n) = v.downcast::<NoteView>() {
        n.update(app, |n, cx| n.set_rendered(p.rendered, cx));
    } else {
        return Err(ApiError::invalid("view: a Markdown file or a note"));
    }
    Ok(Done {})
}

/// Pane `pane` of the live layout `layout`.
fn pane_of(w: &WsRef, layout: &str, pane: u64, app: &App) -> Result<PaneId, ApiError> {
    live(w, layout, app)?;
    let p = PaneId(pane);
    w.deck.read(app).layout().pane(p).map_err(|_| ApiError::not_found(format!("no pane {pane}")))?;
    Ok(p)
}

pub fn pane_split(cx: &CallCx, p: PaneSplitParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let pane = pane_of(&w, &p.layout, p.pane, app)?;
    let (tabs, active) = w.deck.read(app).layout().pane(pane).map(|x| (x.tabs.len(), x.active)).unwrap_or_default();
    if tabs < 2 {
        return Err(ApiError::invalid("a pane of one tab has nothing to split off"));
    }
    quietly(&w, None, app, |l| l.move_tab(pane, active, Drop::Edge(pane, edge(p.edge))))?;
    say(cx, "split", "a pane", app);
    ui(cx, &w, app)
}

pub fn pane_close(cx: &CallCx, p: PaneParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let pane = pane_of(&w, &p.layout, p.pane, app)?;
    let refs = w.deck.read(app).layout().pane(pane).map(|x| x.tabs.clone()).unwrap_or_default();
    keeps_terminals(&w, &refs, app)?;
    quietly(&w, None, app, |l| refs.iter().try_for_each(|r| l.close_view(r)))?;
    say(cx, "closed", "a pane", app);
    ui(cx, &w, app)
}

pub fn pane_collapse(cx: &CallCx, p: PaneCollapseParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let pane = pane_of(&w, &p.layout, p.pane, app)?;
    let now = w.deck.read(app).layout().pane(pane).is_ok_and(|x| x.collapsed);
    if now != p.collapsed {
        quietly(&w, None, app, |l| l.toggle_collapse(pane))?;
    }
    ui(cx, &w, app)
}

pub fn pane_maximize(cx: &CallCx, p: PaneMaximizeParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let pane = pane_of(&w, &p.layout, p.pane, app)?;
    let l = w.deck.read(app).layout();
    let now = l.window_of(pane).ok().and_then(|id| l.window(id).ok()).is_some_and(|win| win.maximized == Some(pane));
    if now != p.maximized {
        quietly(&w, None, app, |l| l.toggle_maximize(pane))?;
    }
    ui(cx, &w, app)
}

pub fn pane_resize(cx: &CallCx, p: PaneResizeParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    live(&w, &p.layout, app)?;
    if !(p.fraction > 0.0 && p.fraction < 1.0) {
        return Err(ApiError::invalid("fraction: between 0 and 1"));
    }
    quietly(&w, None, app, |l| l.set_boundary(SplitId(p.split), p.index as usize, p.fraction))?;
    ui(cx, &w, app)
}

pub fn pane_move_tab(cx: &CallCx, p: MoveTabParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let to = pane_of(&w, &p.layout, p.pane, app)?;
    let (_, from, i) = find(&w, &p.view, app)?;
    match p.edge {
        Some(e) => quietly(&w, None, app, |l| l.move_tab(from, i, Drop::Edge(to, edge(e))))?,
        None => {
            let len = w.deck.read(app).layout().pane(to).map(|x| x.tabs.len()).unwrap_or_default();
            let at = p.at.map_or(len, |a| (a as usize).min(len));
            quietly(&w, None, app, |l| l.move_tab_to(from, i, to, at))?
        }
    }
    ui(cx, &w, app)
}

/// Detached window `window` of the live layout `layout`.
fn window_of(w: &WsRef, layout: &str, window: u64, app: &App) -> Result<WindowId, ApiError> {
    live(w, layout, app)?;
    let id = WindowId(window);
    w.deck.read(app).layout().window(id).map_err(|_| ApiError::not_found(format!("no window {window}")))?;
    if id == MAIN_WINDOW {
        return Err(ApiError::invalid("the main window isn't detached (closing it quits: not through the API)"));
    }
    Ok(id)
}

pub fn window_attach(cx: &CallCx, p: WindowAttachParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let id = window_of(&w, &p.layout, p.window, app)?;
    let to = pane_of(&w, &p.layout, p.pane, app)?;
    quietly(&w, None, app, |l| l.attach(id, Drop::Into(to)))?;
    ui(cx, &w, app)
}

pub fn window_close(cx: &CallCx, p: WindowParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let id = window_of(&w, &p.layout, p.window, app)?;
    let l = w.deck.read(app).layout();
    let refs: Vec<ViewRef> = l.panes_in(id).unwrap_or_default().into_iter().flat_map(|p| l.pane(p).map(|x| x.tabs.clone()).unwrap_or_default()).collect();
    keeps_terminals(&w, &refs, app)?;
    quietly(&w, None, app, |l| l.close_window(id).map(|_| ()))?;
    say(cx, "closed", "a window", app);
    ui(cx, &w, app)
}

/// A switch shows other panes: it asks while the user's keys are in one, and when
/// either layout has a detached window (a new window may take the keyboard).
pub fn layout_switch(cx: &CallCx, p: IdParams, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let d = w.deck.read(app);
    if d.book.current == p.id {
        return ui(cx, &w, app);
    }
    let target = d.book.layouts.get(&p.id).ok_or_else(|| ApiError::not_found(format!("no layout {}", p.id)))?;
    let windows = d.layout().windows.len() > 1 || target.layout.as_ref().is_some_and(|l| l.windows.len() > 1);
    let keys = crate::window_view::keys(&w.deck, app).ok_or_else(|| ApiError::new(ErrorKind::Busy, "a window is being drawn: try again"))?;
    if !keys.is_empty() || windows {
        return Err(asks("switching layouts takes the screen the user is using"));
    }
    w.deck.update(app, |d, cx| d.switch_layout(&p.id, cx)).map_err(|e| ApiError::new(ErrorKind::Refused, e.to_string()))?;
    say(cx, "switched to layout", &p.id, app);
    ui(cx, &w, app)
}

pub fn settings_set(cx: &CallCx, p: SettingsSetParams, app: &mut App) -> Result<Settings, ApiError> {
    if let Some(theme) = &p.theme
        && !crate::app_settings::settings(app).read(app).themes.iter().any(|t| &t.name == theme)
    {
        return Err(ApiError::not_found(format!("no theme {theme}")));
    }
    if let Some(scale) = p.scale {
        if !(0.5..=3.0).contains(&scale) {
            return Err(ApiError::invalid("scale: between 0.5 and 3"));
        }
        crate::app_settings::set_scale(scale, app);
    }
    if let Some(theme) = &p.theme {
        crate::app_settings::set_theme(theme, app);
    }
    super::handlers::settings_get(cx, NoParams {}, app)
}

/// When each workspace was last rescanned for the API, and each caller last notified.
#[derive(Default)]
struct Paced {
    rescans: HashMap<String, Instant>,
    notified: HashMap<String, Instant>,
}

impl Global for Paced {}

pub fn workspace_rescan(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Done, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let now = app.background_executor().now();
    let paced = app.default_global::<Paced>();
    if paced.rescans.get(&w.id).is_some_and(|t| now.duration_since(*t) < RESCAN_EVERY) {
        return Err(ApiError::new(ErrorKind::RateLimited, format!("a rescan every {} s at most", RESCAN_EVERY.as_secs())));
    }
    paced.rescans.insert(w.id.clone(), now);
    w.services.update(app, |s, cx| s.rescan(None, cx));
    say(cx, "rescanned", &w.name, app);
    Ok(Done {})
}

/// One line without control or direction-changing characters, cut to `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let bidi = |c: char| matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{061c}');
    let line: String = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    let clean: String = line.chars().filter(|c| !c.is_control() && !bidi(*c)).collect();
    match clean.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &clean[..i]),
        None => clean,
    }
}

pub fn ui_notify(cx: &CallCx, p: NotifyParams, app: &mut App) -> Result<Notified, ApiError> {
    let text = one_line(&p.text, NOTIFY_CHARS);
    if text.is_empty() {
        return Err(ApiError::invalid("text: nothing to say"));
    }
    let label = cx.who.label();
    let now = app.background_executor().now();
    let paced = app.default_global::<Paced>();
    if paced.notified.get(&label).is_some_and(|t| now.duration_since(*t) < NOTIFY_EVERY) {
        return Err(ApiError::new(ErrorKind::RateLimited, format!("a toast every {} s at most", NOTIFY_EVERY.as_secs())));
    }
    paced.notified.insert(label.clone(), now);
    let kind = match p.kind {
        NoticeKind::Info => devdeck_ui::ToastKind::Info,
        NoticeKind::Warn => devdeck_ui::ToastKind::Warn,
    };
    let id = devdeck_ui::toasts(app).update(app, |t, cx| t.push(kind, format!("{label}: {text}"), vec![], cx));
    Ok(Notified { id })
}

pub fn ui_toasts(_: &CallCx, _: NoParams, app: &mut App) -> Result<Toasts, ApiError> {
    let toasts = devdeck_ui::toasts(app).read(app).queue.history.iter().map(|t| ToastItem { id: t.id, kind: format!("{:?}", t.kind).to_lowercase(), text: t.text.as_ref().into() }).collect();
    Ok(Toasts { toasts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::handlers::view as api_view;

    #[test]
    fn a_notice_is_one_clean_line() {
        assert_eq!(one_line("build\n\n  done\u{202e}evil\x07", 200), "build doneevil");
        assert_eq!(one_line(&"x".repeat(250), 200), format!("{}…", "x".repeat(200)));
        assert_eq!(one_line(" \n ", 200), "");
    }

    #[test]
    fn every_view_devdeck_names_maps_back() {
        for a in [ViewAddr::App { app: "a".into() }, ViewAddr::File { path: "/x".into() }, ViewAddr::Terminal { n: 2, dir: "/d".into() }, ViewAddr::Matrix] {
            assert_eq!(addr(&api_view(a.clone())).unwrap(), a);
        }
        assert!(addr(&View::Unknown).is_err());
    }
}
