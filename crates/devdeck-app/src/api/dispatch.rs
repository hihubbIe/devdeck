//! Calls, on the UI thread: the caller's policy for the method, then its handler. Every
//! method of the catalog has a handler here (a test checks it); the handlers take typed
//! parameters and give typed results, now or later ([`Answer::Later`]: a wait, a file
//! read in the background), from a task the UI thread runs.

use super::Who;
use super::handlers as h;
use devdeck_api::methods::*;
use devdeck_api::{ApiError, ErrorKind, Method, MethodInfo, Policy};
use gpui::{App, Task};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// What a handler knows of the call.
pub struct CallCx<'a> {
    pub who: &'a Who,
    /// This run of devdeck (`devdeck.info`, `session.hello`).
    pub instance: &'a str,
    /// `main`, `demo` or `other`.
    pub state: &'static str,
    /// The connection the call came on (events follow it); none in tests calling here.
    pub conn: Option<u64>,
}

/// A method's answer: now, or from a task the UI thread runs.
pub enum Answer {
    Now(Result<Value, ApiError>),
    Later(Task<Result<Value, ApiError>>),
}

type Handler = Box<dyn Fn(&CallCx, Value, &mut App) -> Answer + Send + Sync>;
/// A method's handler, typed.
type Typed<M> = fn(&CallCx, <M as Method>::Params, &mut App) -> Result<<M as Method>::Result, ApiError>;
/// A handler that answers later: what it checks at once, then a task.
type TypedLater<M> = fn(&CallCx, <M as Method>::Params, &mut App) -> Result<Task<Result<<M as Method>::Result, ApiError>>, ApiError>;

fn params<M: Method>(params: Value) -> Result<M::Params, ApiError> {
    serde_json::from_value(params).map_err(|e| ApiError::invalid(format!("{}: {e}", M::NAME)))
}

fn value<M: Method>(r: M::Result) -> Result<Value, ApiError> {
    serde_json::to_value(r).map_err(|e| ApiError::unavailable(format!("{}: {e}", M::NAME)))
}

fn typed<M: Method>(f: Typed<M>) -> (&'static str, Handler) {
    let handler: Handler = Box::new(move |cx, p, app| Answer::Now(params::<M>(p).and_then(|p| f(cx, p, app)).and_then(value::<M>)));
    (M::NAME, handler)
}

fn later<M: Method>(f: TypedLater<M>) -> (&'static str, Handler) {
    let handler: Handler = Box::new(move |cx, p, app| match params::<M>(p).and_then(|p| f(cx, p, app)) {
        Ok(task) => Answer::Later(app.spawn(async move |_| task.await.and_then(value::<M>))),
        Err(e) => Answer::Now(Err(e)),
    });
    (M::NAME, handler)
}

macro_rules! handlers {
    ($( $ty:ident => $kind:ident $f:path, )*) => {
        fn table() -> BTreeMap<&'static str, Handler> {
            BTreeMap::from([ $( $kind::<$ty>($f), )* ])
        }
    };
}

handlers! {
    SessionHello => typed h::session_hello,
    SessionWhoami => typed h::session_whoami,
    ApiDescribe => typed h::api_describe,
    DevdeckInfo => typed h::devdeck_info,
    DevdeckProblems => typed h::devdeck_problems,
    WorkspaceList => typed h::workspace_list,
    WorkspaceGet => typed h::workspace_get,
    AppList => typed h::app_list,
    AppGet => typed h::app_get,
    RunList => typed h::run_list,
    RunGet => typed h::run_get,
    TerminalList => typed h::terminal_list,
    TerminalGet => typed h::terminal_get,
    UiGet => typed h::ui_get,
    LayoutList => typed h::layout_list,
    LayoutGet => typed h::layout_get,
    NoteList => typed h::note_list,
    NoteGet => typed h::note_get,
    EnvList => typed h::env_list,
    EnvGet => typed h::env_get,
    EditModeGet => typed h::editmode_get,
    SettingsGet => typed h::settings_get,
    RunStart => typed h::run_start,
    RunStop => typed h::run_stop,
    RunRestart => typed h::run_restart,
    RunOutput => typed h::run_output,
    RunSearch => typed h::run_search,
    TerminalOutput => typed h::terminal_output,
    TerminalSearch => typed h::terminal_search,
    TerminalScreen => typed h::terminal_screen,
    NoteRead => typed h::note_read,
    FileRead => later h::file_read,
    RunWait => later super::waits::run_wait,
    TerminalWait => later super::waits::terminal_wait,
    JobGet => typed super::waits::job_get,
    JobWait => later super::waits::job_wait,
    JobCancel => typed super::waits::job_cancel,
    JobList => typed super::waits::job_list,
    EventsSubscribe => typed super::events::subscribe,
    EventsUnsubscribe => typed super::events::unsubscribe,
    GitRepos => later super::gitports::git_repos,
    GitStatus_ => later super::gitports::git_status,
    GitBranches => later super::gitports::git_branches,
    PortList_ => later super::gitports::port_list,
    PortGet => later super::gitports::port_get,
    ViewOpen => later super::present::view_open,
    ViewClose => typed super::present::view_close,
    ViewFocus => typed super::present::view_focus,
    FileOpen => later super::present::file_open,
    FileRendered => typed super::present::file_rendered,
    PaneSplit => typed super::present::pane_split,
    PaneClose => typed super::present::pane_close,
    PaneCollapse => typed super::present::pane_collapse,
    PaneMaximize => typed super::present::pane_maximize,
    PaneResize => typed super::present::pane_resize,
    PaneMoveTab => typed super::present::pane_move_tab,
    WindowAttach => typed super::present::window_attach,
    WindowClose => typed super::present::window_close,
    LayoutSwitch => typed super::present::layout_switch,
    SettingsSet => typed super::present::settings_set,
    WorkspaceRescan => typed super::present::workspace_rescan,
    UiNotify => typed super::present::ui_notify,
    UiToasts => typed super::present::ui_toasts,
    TerminalOpen => typed super::organize::terminal_open,
    TerminalStop => typed super::organize::terminal_stop,
    EnvSetCurrent => typed super::organize::env_set_current,
    NoteCreate => typed super::organize::note_create,
    NoteWrite => typed super::organize::note_write,
    NoteAppend => typed super::organize::note_append,
    NoteRename => typed super::organize::note_rename,
    LayoutSaveAs => typed super::organize::layout_save_as,
    LayoutRename => typed super::organize::layout_rename,
    SidebarPin => typed super::organize::sidebar_pin,
    GroupCreate => typed super::organize::group_create,
    GroupRename => typed super::organize::group_rename,
    GroupMove => typed super::organize::group_move,
    GroupPut => typed super::organize::group_put,
    GroupDelete => typed super::organize::group_delete,
}

static HANDLERS: LazyLock<BTreeMap<&'static str, Handler>> = LazyLock::new(table);
static CATALOG: LazyLock<BTreeMap<&'static str, MethodInfo>> = LazyLock::new(|| devdeck_api::catalog().into_iter().map(|m| (m.name, m)).collect());

pub fn info(method: &str) -> Option<&'static MethodInfo> {
    CATALOG.get(method)
}

/// Calls `method` for the caller: refused unless its policy allows it.
pub fn answer(cx: &CallCx, method: &str, params: Value, app: &mut App) -> Answer {
    let m = match info(method) {
        Some(m) => m,
        None => return Answer::Now(Err(ApiError::not_found(format!("no method {method}")))),
    };
    let refused = match super::policy::of(cx.who, m) {
        Policy::Allow => None,
        Policy::Ask => Some(ApiError::new(ErrorKind::ApprovalRequired, format!("{method} needs the user's approval, which comes in a later version"))),
        Policy::Deny | Policy::Unknown => Some(ApiError::new(ErrorKind::PermissionDenied, format!("{} may not call {method}", cx.who.label()))),
    };
    if let Some(e) = refused {
        return Answer::Now(Err(e));
    }
    if let Err(e) = paced(cx.who, m.class, app) {
        return Answer::Now(Err(e));
    }
    match HANDLERS.get(method) {
        Some(handler) => handler(cx, params, app),
        None => Answer::Now(Err(ApiError::unavailable(format!("{method} isn't built yet")))),
    }
}

/// Calls that change something, per caller, at most (R13.5): an agent in a loop doesn't
/// churn the user's screen or data.
pub const CALLS_PER_WINDOW: usize = 30;
pub const CALLS_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Default)]
struct Calls(std::collections::HashMap<String, std::collections::VecDeque<std::time::Instant>>);

impl gpui::Global for Calls {}

/// Counts a call that changes something (present, organize, run); refused past the rate.
fn paced(who: &Who, class: devdeck_api::Class, app: &mut App) -> Result<(), ApiError> {
    use devdeck_api::Class::{Organize, Present, Run};
    if !matches!(class, Present | Organize | Run) {
        return Ok(());
    }
    let now = app.background_executor().now();
    let calls = app.default_global::<Calls>();
    calls.0.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < CALLS_WINDOW));
    let q = calls.0.entry(who.label()).or_default();
    while q.front().is_some_and(|t| now.duration_since(*t) >= CALLS_WINDOW) {
        q.pop_front();
    }
    if q.len() >= CALLS_PER_WINDOW {
        return Err(ApiError::new(ErrorKind::RateLimited, format!("{CALLS_PER_WINDOW} changes every {} s at most", CALLS_WINDOW.as_secs())));
    }
    q.push_back(now);
    Ok(())
}

/// [`answer`] for a method that answers now (tests, and every method but waits and
/// file reads).
pub fn call(cx: &CallCx, method: &str, params: Value, app: &mut App) -> Result<Value, ApiError> {
    match answer(cx, method, params, app) {
        Answer::Now(r) => r,
        Answer::Later(_) => Err(ApiError::invalid(format!("{method} answers later: use `answer`"))),
    }
}

/// Whether every method of the catalog has a handler (and no handler is unlisted).
pub fn handled() -> (Vec<&'static str>, Vec<&'static str>) {
    let missing = CATALOG.keys().filter(|n| !HANDLERS.contains_key(*n)).copied().collect();
    let extra = HANDLERS.keys().filter(|n| !CATALOG.contains_key(*n)).copied().collect();
    (missing, extra)
}
