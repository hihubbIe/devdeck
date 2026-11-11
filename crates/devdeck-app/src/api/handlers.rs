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
                out.push(Place { pulse: layout_id.to_string(), window: w.id.0, pane: p.0 });
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
        a @ (ViewAddr::Agent { .. } | ViewAddr::Agents) => View::Other { reference: a.to_ref().as_str().into() },
        ViewAddr::Other(r) => View::Other { reference: r.into() },
    }
}

pub(super) fn run_state(s: devdeck_core::runs::RunState) -> (RunState, Option<i32>, Option<i32>) {
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
                id: s.id.0,
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
    run_of(cx, &w, &crate::runner::RunKey::new(&p.app, &p.cmd), app)
}

/// A run as the API gives it, with who started it.
fn run_of(cx: &CallCx, w: &WsRef, key: &crate::runner::RunKey, app: &App) -> Result<Run, ApiError> {
    let s = w.services.read(app);
    let runner = s.runner.read(app);
    let key = key.clone();
    let r = runner.run(&key).filter(|_| !key.is_terminal()).ok_or_else(|| ApiError::not_found(format!("no run of {} {}", key.app, key.cmd)))?;
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
        started_by: match &r.starter {
            Starter::User => StartedBy::User,
            Starter::Caller { workspace, n, .. } => StartedBy::Terminal { workspace: workspace.clone(), n: *n },
        },
        yours: starter(cx.who).is_some_and(|me| me == r.starter),
    })
}

// ---- Starting and stopping runs (R13.5) ----

use crate::runner::{RunKey, RunRefusal, Starter};

/// The caller as a starter of runs (a terminal's shell instance); outside callers never
/// get this far (their policy denies the class).
pub(super) fn starter(who: &Who) -> Option<Starter> {
    match who {
        Who::Terminal { workspace, n, uid, starts, .. } => Some(Starter::Caller { workspace: workspace.clone(), n: *n, uid: uid.clone(), starts: *starts }),
        Who::Outside { .. } => None,
    }
}

fn later(what: String) -> ApiError {
    ApiError::new(devdeck_api::ErrorKind::ApprovalRequired, format!("{what}: that needs the user's approval, which comes in a later version of devdeck"))
}

/// The app and command a run method names: one of the app's main commands (scripts like
/// `deploy` or `db:reset` aren't started by a caller without the user's approval).
fn main_command(s: &Services, app: &str, cmd: &str) -> Result<(devdeck_core::apps::App, devdeck_core::apps::Command), ApiError> {
    let a = s.catalog.app(app).filter(|a| !a.id.is_empty()).ok_or_else(|| ApiError::not_found(format!("no app {app}")))?;
    let c = a.command(cmd).ok_or_else(|| ApiError::not_found(format!("no command {cmd} in {app}")))?;
    if c.tier != devdeck_core::apps::Tier::Primary || c.setup {
        return Err(later(format!("{cmd} isn't one of {app}'s main commands")));
    }
    Ok((a.clone(), c.clone()))
}

/// The env a run gets: `asked`, else `default`; only one of the workspace's envs (its
/// name goes into the command line), and a protected one only with the user's approval
/// when the command uses the env.
fn env_for(s: &Services, a: &devdeck_core::apps::App, c: &devdeck_core::apps::Command, asked: Option<String>, default: Option<String>) -> Result<Option<String>, ApiError> {
    let env = asked.or(default);
    if let Some(e) = &env {
        let found = s.catalog.env(e).ok_or_else(|| ApiError::invalid(format!("no env {e} in this workspace")))?;
        let uses_env = c.line.contains("{env}") || a.env != devdeck_core::apps::EnvApply::None;
        if found.protected && uses_env {
            return Err(later(format!("{e} is a protected env")));
        }
    }
    Ok(env)
}

fn refused(why: RunRefusal) -> ApiError {
    ApiError::new(devdeck_api::ErrorKind::Refused, why.to_string())
}

/// The caller may act on `r`: it started it (its shell, now).
fn own(me: &Starter, title: &str, r: &crate::runner::Run) -> Result<(), ApiError> {
    if *me == r.starter {
        return Ok(());
    }
    let by = match &r.starter {
        Starter::User => "the user".to_string(),
        other => other.label(),
    };
    Err(later(format!("{title} was started by {by}")))
}

/// Tells the user what a caller did (a toast, a few per caller at most).
pub(super) fn say(cx: &CallCx, what: &str, title: &str, app: &mut App) {
    crate::api::notice(cx.who, format!("{} {what} {title}", cx.who.label()), app);
}

pub fn run_start(cx: &CallCx, p: RunStartParams, app: &mut App) -> Result<Run, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let me = starter(cx.who).ok_or_else(|| ApiError::new(devdeck_api::ErrorKind::PermissionDenied, "only a devdeck terminal starts runs"))?;
    let key = RunKey::new(&p.app, &p.cmd);
    let (runner, env, title) = {
        let s = w.services.read(app);
        let (a, c) = main_command(s, &p.app, &p.cmd)?;
        let runner = s.runner.read(app);
        let title = runner.title(&key);
        if let Some(r) = runner.run(&key) {
            match r.state {
                devdeck_core::runs::RunState::Running if p.env.is_some() && p.env != r.env => {
                    return Err(ApiError::new(devdeck_api::ErrorKind::Conflict, format!("{title} runs against {}: restart it to change the env", r.env.as_deref().unwrap_or("no env"))));
                }
                devdeck_core::runs::RunState::Running => return run_of(cx, &w, &key, app),
                // Starting it again once stopped: the one who stops it decides.
                devdeck_core::runs::RunState::Stopping => own(&me, &title, r)?,
                _ => {}
            }
        }
        (s.runner.clone(), env_for(s, &a, &c, p.env, s.env.clone())?, title)
    };
    runner.update(app, |r, cx| r.try_start_as(&key, env, me, cx)).map_err(refused)?;
    say(cx, "started", &title, app);
    run_of(cx, &w, &key, app)
}

pub fn run_stop(cx: &CallCx, p: RunParams, app: &mut App) -> Result<Run, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let me = starter(cx.who).ok_or_else(|| ApiError::new(devdeck_api::ErrorKind::PermissionDenied, "only a devdeck terminal stops runs"))?;
    let key = RunKey::new(&p.app, &p.cmd);
    let (runner, title) = {
        let s = w.services.read(app);
        let runner = s.runner.read(app);
        let r = runner.run(&key).filter(|_| !key.is_terminal()).ok_or_else(|| ApiError::not_found(format!("no run of {} {}", p.app, p.cmd)))?;
        if !r.state.is_running() {
            return run_of(cx, &w, &key, app);
        }
        let title = runner.title(&key);
        own(&me, &title, r)?;
        (s.runner.clone(), title)
    };
    runner.update(app, |r, cx| r.stop(&key, cx));
    say(cx, "stopped", &title, app);
    run_of(cx, &w, &key, app)
}

pub fn run_restart(cx: &CallCx, p: RunStartParams, app: &mut App) -> Result<Run, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let me = starter(cx.who).ok_or_else(|| ApiError::new(devdeck_api::ErrorKind::PermissionDenied, "only a devdeck terminal restarts runs"))?;
    let key = RunKey::new(&p.app, &p.cmd);
    let (runner, env, title) = {
        let s = w.services.read(app);
        let (a, c) = main_command(s, &p.app, &p.cmd)?;
        let runner = s.runner.read(app);
        let title = runner.title(&key);
        let run = runner.run(&key);
        if let Some(r) = run.filter(|r| r.state.is_running()) {
            own(&me, &title, r)?;
        }
        // Its own env, so a restart never moves it to the env the user looks at now.
        let default = run.and_then(|r| r.env.clone()).or_else(|| s.env.clone());
        (s.runner.clone(), env_for(s, &a, &c, p.env, default)?, title)
    };
    runner.update(app, |r, cx| r.restart_as(&key, env, me, cx)).map_err(refused)?;
    say(cx, "restarted", &title, app);
    run_of(cx, &w, &key, app)
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

// ---- Screen and pulses ----

pub fn ui_get(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Ui, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let deck = w.deck.read(app);
    let activity = crate::shell::try_shell(app)
        .and_then(|s| s.read(app).workspaces.iter().find(|x| x.id == w.id).map(|x| x.sidebar.read(app).activity().id()))
        .unwrap_or("pulses")
        .to_string();
    Ok(Ui { pulse: deck.book.current.clone(), activity, windows: windows(deck.layout(), Some(deck), app) })
}

pub fn pulse_list(cx: &CallCx, p: InWorkspace, app: &mut App) -> Result<Pulses, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let book = &w.deck.read(app).book;
    let mut pulses: Vec<PulseSummary> = book.layouts.iter().map(|(id, n)| PulseSummary { id: id.clone(), name: n.name.as_str().into(), current: *id == book.current }).collect();
    pulses.sort_by(|a, b| a.name.0.to_lowercase().cmp(&b.name.0.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    Ok(Pulses { pulses })
}

pub fn pulse_get(cx: &CallCx, p: IdParams, app: &mut App) -> Result<Pulse, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let deck = w.deck.read(app);
    let named = deck.book.layouts.get(&p.id).ok_or_else(|| ApiError::not_found(format!("no pulse {}", p.id)))?;
    let current = p.id == deck.book.current;
    // The live layout is the truth for the current one (its saved copy may be stale).
    let windows = match (current, &named.layout) {
        (true, _) => windows(deck.layout(), Some(deck), app),
        (false, Some(l)) => windows(l, None, app),
        (false, None) => Vec::new(),
    };
    Ok(Pulse { id: p.id, name: named.name.as_str().into(), current, windows })
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

pub fn settings_get(_: &CallCx, _: NoParams, app: &mut App) -> Result<Settings, ApiError> {
    let s = crate::app_settings::settings(app).read(app);
    Ok(Settings { scale: s.store.settings.scale(), theme: s.store.settings.theme.clone() })
}

// ---- Reading output, the screen, notes and files (R13.9) ----

use devdeck_core::terminal::{ReadFrom, Terminal as CoreTerminal, cut_line};

/// Lines one answer holds at most, and bytes of their JSON: small enough for a model's
/// budget once a client wraps it (Claude Code cuts tool results near 25 000 tokens).
const MAX_LINES: usize = 2000;
const MAX_BYTES: usize = 32 * 1024;
/// A line longer than this is cut (a minified file, a progress bar's line).
const LINE_CAP: usize = 2000;
const DEFAULT_TAIL: usize = 100;
const MAX_CONTEXT: usize = 5;
const DEFAULT_MATCHES: usize = 50;
const MAX_MATCHES: usize = 200;
/// Files larger than this aren't read through the API.
const MAX_FILE: u64 = 2 * 1024 * 1024;
const DEFAULT_FILE_LINES: usize = 400;

/// Bytes `text` takes in an answer (JSON-escaped, quotes and a comma).
fn json_len(text: &str) -> usize {
    serde_json::to_string(text).map_or(text.len(), |s| s.len()) + 1
}

/// What a place in some output holds for: this devdeck, this run or terminal, this start
/// of it, this size, the normal screen. Anything else makes it `expired`.
struct Epoch {
    instance: String,
    target: u64,
    starts: u32,
    cols: u16,
    rows: u16,
}

impl Epoch {
    fn of(cx: &CallCx, target: &str, t: &CoreTerminal, starts: u32) -> Self {
        let size = t.size();
        Self { instance: cx.instance.to_string(), target: devdeck_core::fnv1a(target.as_bytes()), starts, cols: size.cols, rows: size.rows }
    }

    fn cursor(&self, at: u64, print: Option<u64>) -> String {
        format!("2.{}.{:x}.{}.{}.{}.{at}.{:x}", self.instance, self.target, self.starts, self.cols, self.rows, print.unwrap_or(0))
    }

    /// The place in `since`, and the fingerprint of the row before it.
    fn place(&self, since: &str) -> Result<(u64, u64), ApiError> {
        let bad = || ApiError::invalid("since: not a cursor this devdeck gave");
        let p: Vec<&str> = since.split('.').collect();
        let [v, inst, target, starts, cols, rows, at, print] = p.as_slice() else { return Err(bad()) };
        if *v != "2" {
            return Err(bad());
        }
        let at: u64 = at.parse().map_err(|_| bad())?;
        let print = u64::from_str_radix(print, 16).map_err(|_| bad())?;
        let same = *inst == self.instance
            && u64::from_str_radix(target, 16).ok() == Some(self.target)
            && starts.parse::<u32>().ok() == Some(self.starts)
            && cols.parse::<u16>().ok() == Some(self.cols)
            && rows.parse::<u16>().ok() == Some(self.rows);
        if !same {
            return Err(expired());
        }
        Ok((at, print))
    }
}

pub(super) fn expired() -> ApiError {
    ApiError::new(devdeck_api::ErrorKind::Expired, "it restarted, its terminal was resized, or devdeck restarted since: read the last lines again")
}

pub(super) fn output_of(cx: &CallCx, target: &str, t: &CoreTerminal, starts: u32, tail: Option<u32>, since: Option<&str>) -> Result<Output, ApiError> {
    let epoch = Epoch::of(cx, target, t, starts);
    let alt = t.modes().alt_screen;
    let from = match since {
        // A full-screen program's screen has no history to go on from.
        Some(_) if alt => return Err(expired()),
        Some(s) => {
            let (at, print) = epoch.place(s)?;
            // The row before the place must still be the one it was (else the count drifted).
            if print != 0 && at > t.dropped() && t.row_print(at - 1).is_some_and(|p| p != print) {
                return Err(expired());
            }
            ReadFrom::Since(at)
        }
        None => ReadFrom::Tail(tail.map_or(DEFAULT_TAIL, |n| n as usize)),
    };
    let s = t.read(from, MAX_LINES, MAX_BYTES, LINE_CAP);
    let next = if alt { String::new() } else { epoch.cursor(s.next, s.next.checked_sub(1).and_then(|r| t.row_print(r))) };
    Ok(Output { lines: s.lines.into_iter().map(Untrusted).collect(), next, truncated: s.truncated, missed: s.missed, partial: s.partial, alt_screen: alt })
}

fn search_in(t: &CoreTerminal, pattern: &str, context: Option<u32>, max: Option<u32>) -> Result<Matches, ApiError> {
    let re = regex::RegexBuilder::new(pattern).size_limit(1 << 20).dfa_size_limit(1 << 20).build().map_err(|e| ApiError::invalid(format!("pattern: {e}")))?;
    let context = context.map_or(0, |c| c as usize).min(MAX_CONTEXT);
    let max = max.map_or(DEFAULT_MATCHES, |m| m as usize).clamp(1, MAX_MATCHES);
    let lines: Vec<String> = t.logical_lines().into_iter().map(|(_, l)| cut_line(&l, LINE_CAP)).collect();
    let mut matches = Vec::new();
    let mut bytes = 0;
    let mut truncated = false;
    for (i, line) in lines.iter().enumerate().filter(|(_, l)| re.is_match(l)) {
        let before: Vec<Untrusted> = lines[i.saturating_sub(context)..i].iter().map(|l| l.as_str().into()).collect();
        let after: Vec<Untrusted> = lines[i + 1..(i + 1 + context).min(lines.len())].iter().map(|l| l.as_str().into()).collect();
        let size = json_len(line) + before.iter().chain(&after).map(|l| json_len(&l.0)).sum::<usize>();
        if matches.len() >= max || bytes + size > MAX_BYTES {
            truncated = true;
            break;
        }
        bytes += size;
        matches.push(Match { text: line.as_str().into(), before, after });
    }
    Ok(Matches { matches, truncated })
}

/// A run's terminal (a command, never a shell), its start count, and what names it.
pub(super) fn with_run<T>(cx: &CallCx, ws: Option<&str>, app: &str, cmd: &str, a: &App, f: impl FnOnce(&CoreTerminal, u32, &str) -> Result<T, ApiError>) -> Result<T, ApiError> {
    let w = workspace(cx, ws, a)?;
    let key = RunKey::new(app, cmd);
    let runner = w.services.read(a).runner.read(a);
    let r = runner.run(&key).filter(|_| !key.is_terminal()).ok_or_else(|| ApiError::not_found(format!("no run of {app} {cmd}")))?;
    f(&r.terminal, r.runs, &format!("run/{app}/{cmd}"))
}

/// Terminal `n`'s terminal, its shell's start count and its uid: the caller's own
/// terminal only (the user's shells hold what the user typed and printed).
pub(super) fn with_terminal<T>(cx: &CallCx, ws: Option<&str>, n: u32, a: &App, f: impl FnOnce(&CoreTerminal, u32, &str) -> Result<T, ApiError>) -> Result<T, ApiError> {
    let w = workspace(cx, ws, a)?;
    let key = RunKey::terminal(n);
    let runner = w.services.read(a).runner.read(a);
    let r = runner.run(&key).ok_or_else(|| ApiError::not_found(format!("no terminal {n}")))?;
    // The caller's own shell, or one it opened (`terminal.open`).
    let mine = matches!(cx.who, Who::Terminal { workspace, n: own, .. } if *workspace == w.id && *own == n) || starter(cx.who).is_some_and(|me| r.starter == me);
    if !mine {
        return Err(later(format!("terminal {n} is the user's")));
    }
    let uid = runner.terminal_uid(&key).unwrap_or_default().to_string();
    f(&r.terminal, r.runs, &uid)
}

pub fn run_output(cx: &CallCx, p: OutputParams, app: &mut App) -> Result<Output, ApiError> {
    with_run(cx, p.workspace.as_deref(), &p.app, &p.cmd, app, |t, starts, target| output_of(cx, target, t, starts, p.tail, p.since.as_deref()))
}

pub fn run_search(cx: &CallCx, p: SearchParams, app: &mut App) -> Result<Matches, ApiError> {
    with_run(cx, p.workspace.as_deref(), &p.app, &p.cmd, app, |t, _, _| search_in(t, &p.pattern, p.context, p.max))
}

pub fn terminal_output(cx: &CallCx, p: TerminalOutputParams, app: &mut App) -> Result<Output, ApiError> {
    with_terminal(cx, p.workspace.as_deref(), p.n, app, |t, starts, target| output_of(cx, target, t, starts, p.tail, p.since.as_deref()))
}

pub fn terminal_search(cx: &CallCx, p: TerminalSearchParams, app: &mut App) -> Result<Matches, ApiError> {
    with_terminal(cx, p.workspace.as_deref(), p.n, app, |t, _, _| search_in(t, &p.pattern, p.context, p.max))
}

pub fn terminal_screen(cx: &CallCx, p: TerminalParams, app: &mut App) -> Result<Screen, ApiError> {
    with_terminal(cx, p.workspace.as_deref(), p.n, app, |t, _, _| {
        let (rows, (row, col)) = t.screen();
        Ok(Screen { rows: rows.into_iter().map(|r| Untrusted(cut_line(&r, LINE_CAP))).collect(), cursor_row: row as u32, cursor_col: col as u32 })
    })
}

pub fn note_read(cx: &CallCx, p: IdParams, app: &mut App) -> Result<NoteText, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let notes = w.services.read(app).notes.clone();
    if notes.read(app).meta(&p.id).is_none() {
        return Err(ApiError::not_found(format!("no note {}", p.id)));
    }
    let text = notes.update(app, |n, _| n.text(&p.id));
    if let Some(why) = notes.read(app).unreadable(&p.id) {
        return Err(ApiError::new(devdeck_api::ErrorKind::Refused, format!("the note can't be read: {why}")));
    }
    // Cut to the budget, at a character's edge (escaping counted).
    let mut end = text.len();
    while json_len(&text[..end]) > MAX_BYTES {
        end = (end * 3 / 4).min(end.saturating_sub(1));
        while !text.is_char_boundary(end) {
            end -= 1;
        }
    }
    Ok(NoteText { id: p.id, truncated: end < text.len(), text: text[..end].into() })
}

/// Names of files never read through the API, whatever folder (secrets; Claude Code's
/// own rules don't reach what devdeck reads for it).
pub(super) fn secret(path: &std::path::Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let sample = [".example", ".sample", ".template", ".dist"].iter().any(|s| name.ends_with(s));
    let env = (name == ".env" || name.starts_with(".env.") || name.ends_with(".env") || name == ".envrc") && !sample;
    let keys = [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore", ".kubeconfig", ".tfvars", ".tfstate", ".tfstate.backup"].iter().any(|e| name.ends_with(e));
    let files = [".npmrc", ".netrc", ".pgpass", ".git-credentials", "credentials", "credentials.json", "kubeconfig", ".htpasswd", ".pypirc", ".vault-token"].contains(&name.as_str());
    let named = ["id_rsa", "id_ed25519", "id_ecdsa", "id_dsa", "service-account"].iter().any(|k| name.starts_with(k));
    let dirs = path.components().any(|c| [".ssh", ".aws", ".gnupg", ".kube", ".docker", ".password-store", ".git"].contains(&c.as_os_str().to_string_lossy().as_ref()));
    env || keys || files || named || dirs
}

/// devdeck's own folders (config, notes, state, its sockets): never read through the
/// API (other workspaces' notes, the audit).
pub(super) fn devdecks_own() -> Vec<std::path::PathBuf> {
    let mut out = vec![devdeck_core::settings::default_config_dir(), devdeck_core::notes::default_data_dir(), devdeck_core::state::default_state_dir()];
    if let Ok(rt) = devdeck_sys::dirs::default_runtime() {
        out.push(rt);
    }
    out.into_iter().filter_map(|p| p.canonicalize().ok()).collect()
}

/// Where `file.read` may read: the workspace's apps' folders (not hidden ones), never
/// `/`, the home folder or one above it.
pub(super) fn file_roots(s: &Services) -> Vec<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).and_then(|h| h.canonicalize().ok());
    let hidden = |id: &str| s.config.as_ref().is_some_and(|c| c.apps.get(id).is_some_and(|o| o.hidden));
    s.catalog
        .apps
        .iter()
        .filter(|a| !a.id.is_empty() && !hidden(&a.id))
        .filter_map(|a| a.dir.canonicalize().ok())
        .filter(|d| d.parent().is_some() && home.as_ref().is_none_or(|h| !h.starts_with(d)))
        .collect()
}

/// May `path` (links resolved) be read?
pub(super) fn readable(path: &std::path::Path, roots: &[std::path::PathBuf], own: &[std::path::PathBuf]) -> Result<(), ApiError> {
    let denied = |why: &str| Err(ApiError::new(devdeck_api::ErrorKind::PermissionDenied, format!("{}: {why}", path.display())));
    if own.iter().any(|o| path.starts_with(o)) {
        return denied("devdeck's own files");
    }
    if !roots.iter().any(|r| path.starts_with(r)) {
        return denied("not in one of the workspace's apps");
    }
    if secret(path) {
        return denied("it may hold secrets");
    }
    // What git ignores is where secrets usually live (and build output nobody reads).
    // When git can't tell, it isn't read.
    match devdeck_core::git::ignored(path) {
        devdeck_core::git::Ignored::No => Ok(()),
        devdeck_core::git::Ignored::Yes => denied("git ignores it, so it may hold secrets"),
        devdeck_core::git::Ignored::Unknown(why) => denied(&format!("git couldn't tell whether it's ignored ({why})")),
    }
}

/// The scope is the UI thread's; the file is read in the background (a hung network
/// mount can't stall devdeck).
pub fn file_read(cx: &CallCx, p: FileParams, app: &mut App) -> Result<gpui::Task<Result<FileLines, ApiError>>, ApiError> {
    let w = workspace(cx, p.workspace.as_deref(), app)?;
    let roots = file_roots(w.services.read(app));
    let own = devdecks_own();
    Ok(app.background_executor().spawn(async move { read_file(&p, &roots, &own) }))
}

fn read_file(p: &FileParams, roots: &[std::path::PathBuf], own: &[std::path::PathBuf]) -> Result<FileLines, ApiError> {
    use std::io::Read;
    let (roots, own) = (roots.to_vec(), own.to_vec());
    let asked = std::path::Path::new(&p.path);
    if !asked.is_absolute() {
        return Err(ApiError::invalid("path: absolute"));
    }
    // Links and `..` resolved: what's checked is where the file really is.
    let path = asked.canonicalize().map_err(|e| ApiError::not_found(format!("{}: {e}", p.path)))?;
    readable(&path, &roots, &own)?;
    // Without blocking (a FIFO), without following a link swapped in since.
    let mut file = devdeck_sys::fs::no_follow(std::fs::OpenOptions::new().read(true), true)
        .open(&path)
        .map_err(|e| ApiError::new(devdeck_api::ErrorKind::Refused, format!("{}: {e}", path.display())))?;
    let meta = file.metadata().map_err(|e| ApiError::new(devdeck_api::ErrorKind::Refused, e.to_string()))?;
    if !meta.is_file() {
        return Err(ApiError::new(devdeck_api::ErrorKind::Refused, format!("{}: not a regular file", path.display())));
    }
    // The file opened is the one checked.
    let opened = devdeck_sys::fs::open_path(&file).map_err(|e| ApiError::new(devdeck_api::ErrorKind::Refused, e.to_string()))?;
    readable(&opened, &roots, &own)?;
    if meta.len() > MAX_FILE {
        return Err(ApiError::new(devdeck_api::ErrorKind::Refused, format!("{} is {} bytes: more than the API reads ({MAX_FILE})", path.display(), meta.len())));
    }
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_FILE).read_to_end(&mut bytes).map_err(|e| ApiError::new(devdeck_api::ErrorKind::Refused, e.to_string()))?;
    if bytes.contains(&0) {
        return Err(ApiError::new(devdeck_api::ErrorKind::Refused, format!("{}: not text", path.display())));
    }
    let text = String::from_utf8(bytes).map_err(|_| ApiError::new(devdeck_api::ErrorKind::Refused, format!("{}: not UTF-8 text", path.display())))?;
    let all: Vec<&str> = text.lines().collect();
    let first = p.from_line.unwrap_or(1).max(1) as usize;
    let want = p.lines.map_or(DEFAULT_FILE_LINES, |n| n as usize).min(MAX_LINES);
    let mut lines = Vec::new();
    let mut size = 0;
    for l in all.iter().skip(first - 1).take(want) {
        let l = cut_line(l, LINE_CAP);
        size += json_len(&l);
        if size > MAX_BYTES {
            break;
        }
        lines.push(Untrusted(l));
    }
    let truncated = first - 1 + lines.len() < all.len();
    Ok(FileLines { path: path.display().to_string().into(), first_line: first as u32, lines, total_lines: all.len() as u32, truncated })
}

#[cfg(test)]
mod tests {
    use super::secret;
    use std::path::Path;

    #[test]
    fn secret_files_are_known_by_name() {
        for p in [
            "/w/a/.env",
            "/w/a/.env.local",
            "/w/a/prod.env",
            "/w/a/.envrc",
            "/w/a/server.pem",
            "/w/a/tls.key",
            "/w/a/.npmrc",
            "/w/a/.pypirc",
            "/w/a/.vault-token",
            "/w/a/prod.tfvars",
            "/w/a/terraform.tfstate",
            "/w/a/service-account-ci.json",
            "/w/a/.git/config",
            "/home/u/.ssh/config",
            "/w/a/id_ed25519.pub",
            "/w/a/.kube/config",
        ] {
            assert!(secret(Path::new(p)), "{p}");
        }
        for p in ["/w/a/.env.example", "/w/a/prod.env.sample", "/w/a/src/main.rs", "/w/a/keys.ts", "/w/a/environment.md", "/w/a/.gitignore"] {
            assert!(!secret(Path::new(p)), "{p}");
        }
    }
}
