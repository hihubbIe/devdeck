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
