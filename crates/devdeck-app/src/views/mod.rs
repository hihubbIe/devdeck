//! The view registry (R3.3): a [`ViewRef`] names a view, [`build`] makes it and
//! [`title`] labels its tab. Adding a view type means a module here and one arm in each
//! of those two functions; the layout engine never changes.
//!
//! Views get their workspace through [`ViewCx`]. Views that take keys are focusable; the
//! window focuses the active view of the focused pane.

pub mod app_view;
pub mod demo;
pub mod run_view;

use crate::deck::Deck;
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

/// What a view knows about its workspace.
#[derive(Clone)]
pub struct ViewCx {
    pub services: Entity<Services>,
    /// The deck the view lives in (to open other views next to it).
    pub deck: WeakEntity<Deck>,
}

impl ViewCx {
    /// Opens (or activates) a view in the main window's focused pane.
    pub fn open(&self, addr: ViewAddr, cx: &mut App) {
        if let Some(deck) = self.deck.upgrade() {
            deck.update(cx, |d, cx| {
                d.apply(cx, |l| {
                    let p = l.main().focused;
                    l.open(addr.to_ref(), p).map(|_| ())
                });
            });
        }
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
        (ViewAddr::Other(_), _) => Built { view: demo::build(r, cx), focus: None },
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
        (_, Some(_)) => message("This view comes later in M1.".into(), cx),
    }
}

/// The tab label of a view.
pub fn title(r: &ViewRef) -> String {
    match ViewAddr::parse(r) {
        ViewAddr::App { app } => app,
        ViewAddr::Run { app, cmd } => format!("{app} · {cmd}"),
        ViewAddr::Svc { env, name } => format!("{env} › {name}"),
        ViewAddr::Logs { env, name } => format!("{env} › {name} · logs"),
        ViewAddr::Pod { env, pod } => format!("{env} › {pod}"),
        ViewAddr::Matrix => "branch matrix".into(),
        ViewAddr::Port { port } => format!(":{port}"),
        ViewAddr::Other(_) => demo::title(r),
    }
}

/// Toggles follow on any view that follows its output (alt+f).
pub fn toggle_follow(view: &AnyView, cx: &mut App) {
    if let Ok(v) = view.clone().downcast::<LogsView>() {
        v.update(cx, |v, cx| v.toggle_follow(cx));
    } else if let Ok(v) = view.clone().downcast::<run_view::RunView>() {
        v.update(cx, |v, cx| v.toggle_follow(cx));
    }
}

/// Glyph and color of a run state (DESIGN visual language).
pub fn run_glyph(state: Option<RunState>, t: &Theme) -> (String, Hsla) {
    match state {
        Some(RunState::Running) => ("●".into(), t.ok),
        Some(RunState::Stopping) => ("◌".into(), t.warn),
        Some(RunState::Exited(0)) => ("○".into(), t.mute),
        Some(RunState::Exited(code)) => (format!("✖ {code}"), t.err),
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
