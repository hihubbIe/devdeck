//! The view registry (R3.3): a [`ViewRef`] names a view, [`build`] makes it and
//! [`title`] labels its tab. Adding a view type means a module here and one arm in each
//! of those two functions; the layout engine never changes.
//!
//! Views get their workspace through [`ViewCx`]. Views that take keys are focusable; the
//! window focuses the active view of the focused pane.

pub mod app_view;
pub mod demo;
pub mod info_view;
pub mod logs_view;
pub mod matrix_view;
pub mod pod_view;
pub mod port_view;
pub mod run_view;
pub mod service_view;

use crate::deck::{Deck, DeckEvent};
use crate::services::Services;
use devdeck_core::layout::ViewRef;
use devdeck_core::runs::RunState;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{Theme, label, theme};
use gpui::{
    AnyView, App, AppContext, Entity, FocusHandle, Focusable, Hsla, IntoElement, ParentElement, Render, SharedString, Styled, WeakEntity,
    Window, div,
};

pub use demo::LogsView;

/// What a view knows about its workspace and itself.
#[derive(Clone)]
pub struct ViewCx {
    pub services: Entity<Services>,
    /// The deck the view lives in (to open other views next to it).
    pub deck: WeakEntity<Deck>,
    /// The address the view was built for.
    pub view: ViewRef,
}

impl ViewCx {
    /// Calls `f(view, true)` when the view comes on screen and `f(view, false)` when it
    /// leaves (another tab in front, pane collapsed or covered, workspace hidden, window
    /// closed). Only changes are reported; the first one comes once the view is shown.
    pub fn on_visibility<V: 'static>(&self, cx: &mut gpui::Context<V>, f: impl Fn(&mut V, bool, &mut gpui::Context<V>) + 'static) -> Option<gpui::Subscription> {
        let deck = self.deck.upgrade()?;
        let me = self.view.clone();
        Some(cx.subscribe(&deck, move |this, _, e: &DeckEvent, cx| {
            let DeckEvent::Visibility { shown, hidden } = e;
            if shown.contains(&me) {
                f(this, true, cx);
            } else if hidden.contains(&me) {
                f(this, false, cx);
            }
        }))
    }

    /// Opens (or activates) a view in the main window's focused pane.
    pub fn open(&self, addr: ViewAddr, cx: &mut App) {
        open_in(&self.deck, addr, cx);
    }
}

/// Opens (or activates) a view in a deck's main window, focused pane.
pub fn open_in(deck: &WeakEntity<Deck>, addr: ViewAddr, cx: &mut App) {
    if let Some(deck) = deck.upgrade() {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let p = l.main().focused;
                l.open(addr.to_ref(), p).map(|_| ())
            });
        });
    }
}

pub struct Built {
    pub view: AnyView,
    /// Where keys go when the view's pane has focus.
    pub focus: Option<FocusHandle>,
}

fn focusable<V: Render + Focusable>(view: Entity<V>, cx: &App) -> Built {
    let focus = Some(view.read(cx).focus_handle(cx));
    Built { view: view.into(), focus }
}

pub fn build(r: &ViewRef, ctx: Option<&ViewCx>, cx: &mut App) -> Built {
    let addr = ViewAddr::parse(r);
    let message = |text: String, cx: &mut App| Built { view: cx.new(|_| Message(text.into())).into(), focus: None };
    match (addr, ctx) {
        (ViewAddr::Other(_), ctx) => demo::build(r, ctx, cx),
        (_, None) => message("This view needs a workspace.".into(), cx),
        (ViewAddr::App { app }, Some(ctx)) => {
            if ctx.services.read(cx).catalog.app(&app).is_none() {
                return message(format!("{app} isn't an app of this workspace."), cx);
            }
            let v = cx.new(|cx| app_view::AppView::new(app, ctx.clone(), cx));
            focusable(v, cx)
        }
        (ViewAddr::Run { app, cmd }, Some(ctx)) => {
            let known = ctx.services.read(cx).catalog.app(&app).is_some_and(|a| a.command(&cmd).is_some());
            if !known {
                return message(format!("{app} has no command {cmd}."), cx);
            }
            let v = cx.new(|cx| run_view::RunView::new(app, cmd, ctx.clone(), cx));
            focusable(v, cx)
        }
        (ViewAddr::Svc { env, name }, Some(ctx)) => {
            let Some(cluster) = ctx.services.read(cx).cluster.clone() else {
                return message("This workspace has no cluster.".into(), cx);
            };
            if cluster.read(cx).env(&env).is_none() {
                return message(format!("{env} isn't an env of this workspace."), cx);
            }
            let v = cx.new(|cx| service_view::ServiceView::new(env, name, ctx.clone(), cluster, cx));
            focusable(v, cx)
        }
        (ViewAddr::Logs { env, name }, Some(ctx)) => match cluster_env(ctx, &env, cx) {
            Err(text) => message(text, cx),
            Ok(cluster) => {
                let v = cx.new(|cx| logs_view::ServiceLogsView::new(env, name, ctx.clone(), cluster, cx));
                focusable(v, cx)
            }
        },
        (ViewAddr::Pod { env, pod }, Some(ctx)) => match cluster_env(ctx, &env, cx) {
            Err(text) => message(text, cx),
            Ok(cluster) => Built { view: cx.new(|cx| pod_view::PodView::new(env, pod, ctx.clone(), cluster, cx)).into(), focus: None },
        },
        (ViewAddr::Matrix, Some(ctx)) => match ctx.services.read(cx).cluster.clone() {
            None => message("This workspace has no cluster.".into(), cx),
            Some(cluster) => {
                let v = cx.new(|cx| matrix_view::MatrixView::new(ctx.clone(), cluster, cx));
                focusable(v, cx)
            }
        },
        (ViewAddr::Port { port }, Some(ctx)) => Built { view: cx.new(|cx| port_view::PortView::new(port, ctx.clone(), cx)).into(), focus: None },
        (ViewAddr::Problems, Some(ctx)) => Built { view: cx.new(|cx| info_view::ProblemsView::new(ctx.clone(), cx)).into(), focus: None },
        (ViewAddr::Welcome, Some(_)) => Built { view: cx.new(|_| info_view::WelcomeView).into(), focus: None },
    }
}

/// The workspace's cluster when it has `env`, or the message to show instead.
fn cluster_env(ctx: &ViewCx, env: &str, cx: &App) -> Result<Entity<crate::cluster::Cluster>, String> {
    let cluster = ctx.services.read(cx).cluster.clone().ok_or("This workspace has no cluster.")?;
    if cluster.read(cx).env(env).is_none() {
        return Err(format!("{env} isn't an env of this workspace."));
    }
    Ok(cluster)
}

/// The tab label of a view.
pub fn title(r: &ViewRef) -> String {
    match ViewAddr::parse(r) {
        ViewAddr::App { app } => app,
        ViewAddr::Run { app, cmd } => format!("{app} · {}", devdeck_core::apps::name_of(&cmd)),
        ViewAddr::Svc { env, name } => format!("{env} › {name}"),
        ViewAddr::Logs { env, name } => format!("{env} › {name} · logs"),
        ViewAddr::Pod { env, pod } => format!("{env} › {pod}"),
        ViewAddr::Matrix => "branch matrix".into(),
        ViewAddr::Port { port } => format!(":{port}"),
        ViewAddr::Problems => "problems".into(),
        ViewAddr::Welcome => "welcome".into(),
        ViewAddr::Other(_) => demo::title(r),
    }
}

/// Glyph and color of a run state (DESIGN visual language).
pub fn run_glyph(state: Option<RunState>, t: &Theme) -> (String, Hsla) {
    match state {
        Some(RunState::Running) => ("●".into(), t.ok),
        Some(RunState::Stopping) => ("◌".into(), t.warn),
        Some(RunState::Exited(0)) => ("○".into(), t.mute),
        Some(s @ (RunState::Exited(_) | RunState::Killed(_) | RunState::NotStarted)) => (format!("✖ {}", s.failed().unwrap_or(1)), t.err),
        Some(RunState::Stopped) | None => ("○".into(), t.dim),
    }
}

/// A view that only says something (unknown or unavailable address).
pub struct Message(SharedString);

impl Render for Message {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        div().size_full().flex().items_center().justify_center().text_color(t.mute).child(label(self.0.clone()))
    }
}
