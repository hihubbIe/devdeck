//! Calls, on the UI thread: the caller's policy for the method, then its handler. Every
//! method of the catalog has a handler here (a test checks it); the handlers take typed
//! parameters and give typed results, and only read devdeck's state in phase 1.

use super::Who;
use super::handlers as h;
use devdeck_api::methods::*;
use devdeck_api::{ApiError, ErrorKind, Method, MethodInfo, Policy};
use gpui::App;
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
}

type Handler = Box<dyn Fn(&CallCx, Value, &mut App) -> Result<Value, ApiError> + Send + Sync>;
/// A method's handler, typed.
type Typed<M> = fn(&CallCx, <M as Method>::Params, &mut App) -> Result<<M as Method>::Result, ApiError>;

fn typed<M: Method>(f: Typed<M>) -> (&'static str, Handler) {
    let handler: Handler = Box::new(move |cx, params, app| {
        let p: M::Params = serde_json::from_value(params).map_err(|e| ApiError::invalid(format!("{}: {e}", M::NAME)))?;
        let r = f(cx, p, app)?;
        serde_json::to_value(r).map_err(|e| ApiError::unavailable(format!("{}: {e}", M::NAME)))
    });
    (M::NAME, handler)
}

macro_rules! handlers {
    ($( $ty:ident => $f:path, )*) => {
        fn table() -> BTreeMap<&'static str, Handler> {
            BTreeMap::from([ $( typed::<$ty>($f), )* ])
        }
    };
}

handlers! {
    SessionHello => h::session_hello,
    SessionWhoami => h::session_whoami,
    ApiDescribe => h::api_describe,
    DevdeckInfo => h::devdeck_info,
    DevdeckProblems => h::devdeck_problems,
    WorkspaceList => h::workspace_list,
    WorkspaceGet => h::workspace_get,
    AppList => h::app_list,
    AppGet => h::app_get,
    RunList => h::run_list,
    RunGet => h::run_get,
    TerminalList => h::terminal_list,
    TerminalGet => h::terminal_get,
    UiGet => h::ui_get,
    LayoutList => h::layout_list,
    LayoutGet => h::layout_get,
    NoteList => h::note_list,
    NoteGet => h::note_get,
    EnvList => h::env_list,
    EnvGet => h::env_get,
    EditModeGet => h::editmode_get,
    SettingsGet => h::settings_get,
}

static HANDLERS: LazyLock<BTreeMap<&'static str, Handler>> = LazyLock::new(table);
static CATALOG: LazyLock<BTreeMap<&'static str, MethodInfo>> = LazyLock::new(|| devdeck_api::catalog().into_iter().map(|m| (m.name, m)).collect());

pub fn info(method: &str) -> Option<&'static MethodInfo> {
    CATALOG.get(method)
}

/// Calls `method` for the caller: refused unless its policy allows it.
pub fn call(cx: &CallCx, method: &str, params: Value, app: &mut App) -> Result<Value, ApiError> {
    let m = info(method).ok_or_else(|| ApiError::not_found(format!("no method {method}")))?;
    match super::policy::of(cx.who, m) {
        Policy::Allow => {}
        Policy::Ask => return Err(ApiError::new(ErrorKind::ApprovalRequired, format!("{method} needs the user's approval, which comes in a later version"))),
        Policy::Deny | Policy::Unknown => return Err(ApiError::new(ErrorKind::PermissionDenied, format!("{} may not call {method}", cx.who.label()))),
    }
    let handler = HANDLERS.get(method).ok_or_else(|| ApiError::unavailable(format!("{method} isn't built yet")))?;
    handler(cx, params, app)
}

/// Whether every method of the catalog has a handler (and no handler is unlisted).
pub fn handled() -> (Vec<&'static str>, Vec<&'static str>) {
    let missing = CATALOG.keys().filter(|n| !HANDLERS.contains_key(*n)).copied().collect();
    let extra = HANDLERS.keys().filter(|n| !CATALOG.contains_key(*n)).copied().collect();
    (missing, extra)
}
