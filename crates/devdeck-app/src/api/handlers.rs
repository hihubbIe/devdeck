//! The methods' handlers: each reads devdeck's state on the UI thread and gives a typed
//! result. A terminal caller sees its own workspace only (R2.1); an outside caller
//! reaches only what [`super::policy::OUTSIDE_METHODS`] lists.

use super::dispatch::CallCx;
use super::{Who, policy};
use crate::deck::Deck;
use crate::services::Services;
use devdeck_api::types::*;
use devdeck_api::{API_LEVEL, ApiError, Policy};
use devdeck_core::layout::Layout as CoreLayout;
use devdeck_core::view_addr::ViewAddr;
use gpui::{App, Entity};

/// A workspace a call is about.
pub struct WsRef {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub deck: Entity<Deck>,
    pub services: Entity<Services>,
}

/// The workspace asked for, within the caller's reach: a terminal's own (the default),
/// else for outside the one asked or the active one.
pub fn workspace(cx: &CallCx, asked: Option<&str>, app: &App) -> Result<WsRef, ApiError> {
    let shell = crate::shell::try_shell(app).ok_or_else(|| ApiError::unavailable("devdeck is starting"))?;
    let s = shell.read(app);
    let id = match (cx.who, asked) {
        (Who::Terminal { workspace, .. }, None) => workspace.clone(),
        (Who::Terminal { workspace, .. }, Some(a)) if a == workspace => a.to_string(),
        (Who::Terminal { .. }, Some(a)) => return Err(ApiError::not_found(format!("no workspace {a}"))),
        (Who::Outside { .. }, Some(a)) => a.to_string(),
        (Who::Outside { .. }, None) => s.workspaces.get(s.active).map(|w| w.id.clone()).unwrap_or_default(),
    };
    s.workspaces
        .iter()
        .enumerate()
        .find(|(_, w)| w.id == id)
        .map(|(i, w)| WsRef { id: w.id.clone(), name: w.name.clone(), active: i == s.active, deck: w.deck.clone(), services: w.services.clone() })
        .ok_or_else(|| ApiError::not_found(format!("no workspace {id}")))
}

/// Where views `wanted` show in `layout` (saved under `layout_id`).
pub fn places(layout: &CoreLayout, layout_id: &str, wanted: impl Fn(&ViewAddr) -> bool) -> Vec<Place> {
    let mut out = Vec::new();
    for w in &layout.windows {
        for p in layout.panes_in(w.id).unwrap_or_default() {
            let Ok(pane) = layout.pane(p) else { continue };
            if pane.tabs.iter().any(|v| wanted(&ViewAddr::parse(v))) {
                out.push(Place { layout: layout_id.to_string(), window: w.id.0, pane: p.0 });
            }
        }
    }
    out
}

/// Where terminal `n` shows in its workspace's current layout.
pub fn terminal_places(deck: &Deck, n: u32) -> Vec<Place> {
    places(deck.layout(), &deck.book.current, |a| matches!(a, ViewAddr::Terminal { n: t, .. } if *t == n))
}

pub fn session_hello(cx: &CallCx, p: HelloParams, _: &mut App) -> Result<Hello, ApiError> {
    if let Some(min) = p.api_level_min.filter(|l| *l > API_LEVEL) {
        return Err(ApiError::unavailable(format!("devdeck speaks level {API_LEVEL}, the client needs {min}")));
    }
    Ok(Hello { api_level: API_LEVEL, instance: cx.instance.to_string(), caller: cx.who.caller(), policy: policy::per_class(cx.who) })
}

pub fn session_whoami(cx: &CallCx, _: NoParams, app: &mut App) -> Result<Whoami, ApiError> {
    let caller = cx.who.caller();
    Ok(match cx.who {
        Who::Terminal { workspace: ws, n, uid, .. } => {
            let shown = workspace(cx, None, app).map(|w| terminal_places(w.deck.read(app), *n)).unwrap_or_default();
            Whoami { caller, workspace: Some(ws.clone()), terminal: Some(TerminalRef { n: *n, uid: uid.clone() }), shown }
        }
        Who::Outside { .. } => Whoami { caller, workspace: None, terminal: None, shown: Vec::new() },
    })
}

pub fn api_describe(cx: &CallCx, p: DescribeParams, _: &mut App) -> Result<Describe, ApiError> {
    let one = p.method.is_some();
    let methods: Vec<MethodEntry> = devdeck_api::catalog()
        .into_iter()
        .filter(|m| p.method.as_deref().is_none_or(|n| n == m.name))
        .map(|m| MethodEntry {
            allowed: policy::of(cx.who, &m) == Policy::Allow,
            name: m.name.to_string(),
            summary: m.summary.to_string(),
            class: m.class,
            escalates_to: m.escalates_to.to_vec(),
            since: m.since,
            experimental: m.experimental,
            params: one.then(|| m.params.clone()),
            result: one.then(|| m.result.clone()),
        })
        .collect();
    if let (Some(name), true) = (&p.method, methods.is_empty()) {
        return Err(ApiError::not_found(format!("no method {name}")));
    }
    Ok(Describe { api_level: API_LEVEL, methods })
}

pub fn devdeck_info(cx: &CallCx, _: NoParams, _: &mut App) -> Result<Info, ApiError> {
    Ok(Info {
        name: "devdeck".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api_level: API_LEVEL,
        instance: cx.instance.to_string(),
        state: cx.state.to_string(),
        pid: std::process::id(),
    })
}

// ---- Conversions ----

pub fn view(a: ViewAddr) -> View {
    match a {
        ViewAddr::App { app } => View::App { app },
        ViewAddr::Run { app, cmd } => View::Run { app, cmd },
        ViewAddr::Svc { env, name } => View::Svc { env, name },
        ViewAddr::Logs { env, name } => View::Logs { env, name },
        ViewAddr::Pod { env, pod } => View::Pod { env, pod },
        ViewAddr::Matrix => View::Matrix,
        ViewAddr::Port { port } => View::Port { port },
        ViewAddr::Problems => View::Problems,
        ViewAddr::Welcome => View::Welcome,
        ViewAddr::Terminal { n, dir } => View::Terminal { n, dir: dir.into() },
        ViewAddr::Git { root } => View::Git { root: root.into() },
        ViewAddr::File { path } => View::File { path: path.into() },
        ViewAddr::Note { id } => View::Note { id },
        ViewAddr::Other(r) => View::Other { reference: r.into() },
    }
}

fn run_state(s: devdeck_core::runs::RunState) -> (RunState, Option<i32>, Option<i32>) {
    use devdeck_core::runs::RunState as S;
    match s {
        S::Running => (RunState::Running, None, None),
        S::Stopping => (RunState::Stopping, None, None),
        S::Exited(c) => (RunState::Exited, Some(c), None),
        S::Killed(sig) => (RunState::Killed, None, Some(sig)),
        S::NotStarted => (RunState::NotStarted, None, None),
        S::Stopped => (RunState::Stopped, None, None),
    }
}

/// A layout's windows; with `deck`, its tabs' titles (the live layout's only).
fn windows(l: &CoreLayout, deck: Option<&Deck>, app: &App) -> Vec<Window> {
    use devdeck_core::layout::{Axis as CoreAxis, MAIN_WINDOW, Node as CoreNode};
    fn node(n: &CoreNode, l: &CoreLayout, deck: Option<&Deck>, app: &App) -> Node {
        match n {
            CoreNode::Split(s) => Node::Split {
                axis: match s.axis {
                    CoreAxis::Row => Axis::Row,
                    CoreAxis::Column => Axis::Column,
                },
                children: s.children.iter().map(|c| Child { weight: c.weight, node: node(&c.node, l, deck, app) }).collect(),
            },
            CoreNode::Pane(id) => {
                let Ok(p) = l.pane(*id) else { return Node::Unknown };
                let tabs = p
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(i, r)| Tab { view: view(ViewAddr::parse(r)), title: deck.map(|d| d.title(r, app).into()), active: i == p.active })
                    .collect();
                Node::Pane { pane: id.0, collapsed: p.collapsed, tabs }
            }
        }
    }
    l.windows
        .iter()
        .map(|w| Window { id: w.id.0, main: w.id == MAIN_WINDOW, focused: w.focused.0, maximized: w.maximized.map(|p| p.0), root: node(&w.root, l, deck, app) })
        .collect()
}

fn note(n: &devdeck_core::notes::NoteMeta) -> Note {
    Note { id: n.id.clone(), app: n.app.clone(), name: n.name.as_str().into() }
}

fn envs(s: &Services) -> Vec<Env> {
    s.catalog.envs.iter().map(|e| Env { name: e.name.clone(), protected: e.protected, current: s.env.as_deref() == Some(e.name.as_str()) }).collect()
}

// ---- devdeck, workspaces ----

pub fn devdeck_problems(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Problems, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let mut out = w.services.read(app).all_problems(app);
    out.extend(crate::environment::environment(app).read(app).problems());
    Ok(Problems { problems: out.into_iter().map(Untrusted).collect() })
}

pub fn workspace_list(cx: &CallCx, _: NoParams, app: &mut App) -> Result<Workspaces, ApiError> {
    let shell = crate::shell::try_shell(app).ok_or_else(|| ApiError::unavailable("devdeck is starting"))?;
    let s = shell.read(app);
    let workspaces = s
        .workspaces
        .iter()
        .enumerate()
        .filter(|(_, w)| match cx.who {
            Who::Terminal { workspace, .. } => &w.id == workspace,
            Who::Outside { .. } => true,
        })
        .map(|(i, w)| WorkspaceSummary { id: w.id.clone(), name: w.name.as_str().into(), active: i == s.active })
        .collect();
    Ok(Workspaces { workspaces })
}

pub fn workspace_get(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Workspace, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let s = w.services.read(app);
    let folders = s.config.as_ref().map(|c| c.folders.iter().map(|f| f.display().to_string().into()).collect()).unwrap_or_default();
    Ok(Workspace {
        id: w.id,
        name: w.name.into(),
        active: w.active,
        folders,
        envs: envs(s),
        current_env: s.env.clone(),
        default_env: s.catalog.default_env.clone(),
        cluster: s.catalog.cluster.is_some(),
    })
}

// ---- Apps and runs ----

pub fn app_list(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Apps, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let s = w.services.read(app);
    let runner = s.runner.read(app);
    let mut apps: Vec<AppSummary> = s
        .catalog
        .apps
        .iter()
        .map(|a| AppSummary {
            id: a.id.clone(),
            name: a.name.as_str().into(),
            kind: a.kind.clone(),
            library: a.library,
            running: runner.app_running(&a.id),
            usable: !s.unusable.contains_key(&a.id),
        })
        .collect();
    apps.sort_by(|a, b| a.name.0.to_lowercase().cmp(&b.name.0.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    Ok(Apps { apps })
}

pub fn app_get(cx: &CallCx, p: AppParams, app: &mut App) -> Result<ApiApp, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let s = w.services.read(app);
    let a = s.catalog.app(&p.app).ok_or_else(|| ApiError::not_found(format!("no app {}", p.app)))?;
    let runner = s.runner.read(app);
    let mut commands: Vec<CommandInfo> = a
        .commands
        .iter()
        .map(|c| CommandInfo {
            id: c.id.clone(),
            name: c.name().into(),
            line: c.line.as_str().into(),
            tier: match c.tier {
                devdeck_core::apps::Tier::Primary => Tier::Primary,
                devdeck_core::apps::Tier::Secondary => Tier::Secondary,
                devdeck_core::apps::Tier::Hidden => Tier::Hidden,
            },
            setup: c.setup,
            ports: c.ports.clone(),
            running: runner.state(&crate::runner::RunKey::new(&a.id, &c.id)).is_some_and(|st| st.is_running()),
        })
        .collect();
    commands.sort_by(|x, y| x.id.cmp(&y.id));
    Ok(ApiApp {
        id: a.id.clone(),
        name: a.name.as_str().into(),
        kind: a.kind.clone(),
        library: a.library,
        dir: a.dir.display().to_string().into(),
        commands,
        urls: a.urls.iter().map(|u| u.as_str().into()).collect(),
        unusable: s.unusable.get(&a.id).map(|u| u.as_str().into()),
        notes: s.notes.read(app).of(&a.id).len() as u32,
    })
}

/// `devdeck_api`'s app (named apart from GPUI's `App`).
pub type ApiApp = devdeck_api::types::App;

pub fn run_list(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Runs, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let runner = w.services.read(app).runner.read(app);
    let mut runs: Vec<RunSummary> = runner
        .runs()
        .filter(|(k, _)| !k.is_terminal())
        .map(|(k, r)| RunSummary { app: k.app.clone(), cmd: k.cmd.clone(), title: runner.title(k).into(), state: run_state(r.state).0 })
        .collect();
    runs.sort_by(|a, b| a.title.0.to_lowercase().cmp(&b.title.0.to_lowercase()).then_with(|| (&a.app, &a.cmd).cmp(&(&b.app, &b.cmd))));
    Ok(Runs { runs })
}

pub fn run_get(cx: &CallCx, p: RunParams, app: &mut App) -> Result<Run, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let s = w.services.read(app);
    let runner = s.runner.read(app);
    let key = crate::runner::RunKey::new(&p.app, &p.cmd);
    let r = runner.run(&key).filter(|_| !key.is_terminal()).ok_or_else(|| ApiError::not_found(format!("no run of {} {}", p.app, p.cmd)))?;
    let (state, exit_code, signal) = run_state(r.state);
    let env_vars = s
        .catalog
        .app(&key.app)
        .and_then(|a| Some(a.env.apply(&a.command(&key.cmd)?.line, r.env.as_deref()).vars.into_iter().map(|(n, _)| n).collect()))
        .unwrap_or_default();
    Ok(Run {
        app: key.app.clone(),
        cmd: key.cmd.clone(),
        title: runner.title(&key).into(),
        state,
        exit_code,
        signal,
        error: r.error.as_deref().map(Into::into),
        env: r.env.clone(),
        line: runner.line(&key, r.env.as_deref()).map(Into::into),
        args: runner.args(&key).into(),
        env_vars,
        pid: runner.pids().into_iter().find(|(k, _)| *k == key).map(|(_, p)| p),
        ports: runner.running_ports().into_iter().filter(|(k, _)| *k == key).map(|(_, p)| p).collect(),
        starts: r.runs,
        took_ms: r.took.map(|d| d.as_millis() as u64),
    })
}

// ---- Terminals ----

fn terminal_summary(runner: &crate::runner::Runner, key: &crate::runner::RunKey, dir: &std::path::Path) -> Option<TerminalSummary> {
    Some(TerminalSummary {
        n: key.terminal_number()?,
        uid: runner.terminal_uid(key)?.to_string(),
        dir: dir.display().to_string().into(),
        running: runner.state(key).is_some_and(|s| s.is_running()),
        program: runner.terminal_program(key).map(Into::into),
    })
}

pub fn terminal_list(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Terminals, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let runner = w.services.read(app).runner.read(app);
    let mut terminals: Vec<TerminalSummary> = runner.terminals().iter().filter_map(|(k, d)| terminal_summary(runner, k, d)).collect();
    terminals.sort_by_key(|t| t.n);
    Ok(Terminals { terminals })
}

pub fn terminal_get(cx: &CallCx, p: TerminalParams, app: &mut App) -> Result<Terminal, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let runner = w.services.read(app).runner.read(app);
    let key = crate::runner::RunKey::terminal(p.n);
    let t = runner.terminals().into_iter().find(|(k, _)| *k == key).and_then(|(k, d)| terminal_summary(runner, &k, &d)).ok_or_else(|| ApiError::not_found(format!("no terminal {}", p.n)))?;
    let shown = terminal_places(w.deck.read(app), p.n);
    Ok(Terminal { n: t.n, uid: t.uid, dir: t.dir, running: t.running, program: t.program, shown })
}

// ---- Screen and layouts ----

pub fn ui_get(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let deck = w.deck.read(app);
    Ok(Ui { layout: deck.book.current.clone(), windows: windows(deck.layout(), Some(deck), app) })
}

pub fn layout_list(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Layouts, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let book = &w.deck.read(app).book;
    let mut layouts: Vec<LayoutSummary> = book.layouts.iter().map(|(id, n)| LayoutSummary { id: id.clone(), name: n.name.as_str().into(), current: *id == book.current }).collect();
    layouts.sort_by(|a, b| a.name.0.to_lowercase().cmp(&b.name.0.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    Ok(Layouts { layouts })
}

pub fn layout_get(cx: &CallCx, p: IdParams, app: &mut App) -> Result<Layout, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let deck = w.deck.read(app);
    let named = deck.book.layouts.get(&p.id).ok_or_else(|| ApiError::not_found(format!("no layout {}", p.id)))?;
    let current = p.id == deck.book.current;
    // The live layout is the truth for the current one (its saved copy may be stale).
    let windows = match (current, &named.layout) {
        (true, _) => windows(deck.layout(), Some(deck), app),
        (false, Some(l)) => windows(l, None, app),
        (false, None) => Vec::new(),
    };
    Ok(Layout { id: p.id, name: named.name.as_str().into(), current, windows })
}

// ---- Notes, envs, settings ----

pub fn note_list(cx: &CallCx, p: NoteListParams, app: &mut App) -> Result<Notes, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let notes = w.services.read(app).notes.read(app);
    Ok(Notes { notes: notes.all().iter().filter(|n| p.app.as_deref().is_none_or(|a| a == n.app)).map(note).collect() })
}

pub fn note_get(cx: &CallCx, p: IdParams, app: &mut App) -> Result<Note, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let notes = w.services.read(app).notes.read(app);
    notes.meta(&p.id).map(note).ok_or_else(|| ApiError::not_found(format!("no note {}", p.id)))
}

pub fn env_list(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Envs, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    Ok(Envs { envs: envs(w.services.read(app)) })
}

pub fn env_get(cx: &CallCx, p: EnvParams, app: &mut App) -> Result<Env, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    envs(w.services.read(app)).into_iter().find(|e| e.name == p.name).ok_or_else(|| ApiError::not_found(format!("no env {}", p.name)))
}

pub fn editmode_get(_: &CallCx, _: NoParams, app: &mut App) -> Result<EditMode, ApiError> {
    Ok(EditMode { on: crate::edit_mode::is_on(app) })
}

pub fn settings_get(_: &CallCx, _: NoParams, app: &mut App) -> Result<Settings, ApiError> {
    let s = crate::app_settings::settings(app).read(app);
    Ok(Settings { scale: s.store.settings.scale(), theme: s.store.settings.theme.clone() })
}
