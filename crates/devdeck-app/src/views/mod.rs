//! The view registry (R3.3): a [`ViewRef`] names a view, [`build`] makes it and
//! [`title`] labels its tab. Adding a view type means a module here and one arm in each
//! of those two functions; the layout engine never changes.
//!
//! Views get their workspace through [`ViewCx`]. Views that take keys are focusable; the
//! window focuses the active view of the focused pane. Views with something worth
//! keeping across a restart (scroll, filters, a run that was running) implement
//! [`KeepsState`]: they come back as they were when the workspace reopens.

pub mod app_view;
pub mod demo;
pub mod file_view;
pub mod git_view;
pub mod info_view;
pub mod logs_feed;
pub mod logs_view;
pub mod matrix_view;
pub mod note_view;
pub mod pod_view;
pub mod port_view;
pub mod run_view;
pub mod service_view;

use crate::deck::{Deck, DeckEvent};
use crate::services::Services;
use devdeck_core::layout::ViewRef;
use devdeck_core::runs::RunState;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::{Icon, Theme, dot, icon, label, spinner, theme};
use gpui::{
    AnyElement, AnyView, App, AppContext, ElementId, Entity, FocusHandle, Focusable, IntoElement, Pixels, ParentElement, Render, SharedString, Styled, WeakEntity,
    Window, div,
};

pub use demo::LogsView;

gpui::actions!(view, [
    /// A Markdown file or a note: its source or rendered (R12.4).
    ToggleRendered
]);

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
    /// What the view keeps of itself across a restart.
    pub save: Option<Saver>,
    /// A message standing in for a view whose app, command or env isn't there (yet): the
    /// deck builds it again when the workspace's apps change.
    pub missing: bool,
    /// What the view says about itself beyond its address ([`Live`]).
    pub live: Option<Lively>,
}

/// An action a view offers right now (R5.1, R12.4): the registry's action `id`, drawn
/// with `icon` in its pane's header (or strip, for the active tab) and listed in its
/// tab's menu; `on` when it's a state that is on.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewAction {
    pub id: &'static str,
    pub icon: Icon,
    pub on: bool,
}

/// A view that says more about itself than its address: its tab's title when the
/// address can't say it (a note's name), and the actions it offers now. Whenever the
/// view changes its deck redraws, so the title and the header follow.
pub trait Live: Render {
    fn title(&self, _cx: &App) -> Option<String> {
        None
    }
    fn actions(&self, _cx: &App) -> Vec<ViewAction> {
        Vec::new()
    }
    /// Runs one of its [`actions`](Live::actions) (from its header or tab menu).
    fn run(&mut self, _id: &str, _window: &mut Window, _cx: &mut gpui::Context<Self>) {}
}

type TitleFn = std::rc::Rc<dyn Fn(&App) -> Option<String>>;
type ActionsFn = std::rc::Rc<dyn Fn(&App) -> Vec<ViewAction>>;
type RunFn = std::rc::Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// How the deck reads a [`Live`] view (through a weak handle: never keeps it alive).
#[derive(Clone)]
pub struct Lively {
    title: TitleFn,
    actions: ActionsFn,
    run: RunFn,
}

impl Lively {
    pub fn title(&self, cx: &App) -> Option<String> {
        (self.title)(cx)
    }
    pub fn actions(&self, cx: &App) -> Vec<ViewAction> {
        (self.actions)(cx)
    }
    pub fn run(&self, id: &str, window: &mut Window, cx: &mut App) {
        (self.run)(id, window, cx)
    }
}

/// Adds `view`'s [`Live`] part to `built`; its changes reach its deck (through the
/// observer `keeping` set, else one of its own).
fn live<V: Live>(mut built: Built, view: &Entity<V>, ctx: &ViewCx, cx: &mut App) -> Built {
    if built.save.is_none() {
        let (deck, r) = (ctx.deck.clone(), ctx.view.clone());
        cx.observe(view, move |_, cx| {
            let _ = deck.update(cx, |d, cx| d.view_changed(&r, cx));
        })
        .detach();
    }
    let (t, a, run) = (view.downgrade(), view.downgrade(), view.downgrade());
    built.live = Some(Lively {
        title: std::rc::Rc::new(move |cx| t.upgrade().and_then(|v| v.read(cx).title(cx))),
        actions: std::rc::Rc::new(move |cx| a.upgrade().map(|v| v.read(cx).actions(cx)).unwrap_or_default()),
        run: std::rc::Rc::new(move |id, window, cx| {
            let _ = run.update(cx, |v, cx| v.run(id, window, cx));
        }),
    });
    built
}

/// Reads a view's state to save (`None` once the view is gone).
pub type Saver = std::rc::Rc<dyn Fn(&App) -> Option<serde_json::Value>>;

/// A view that keeps some state across restarts. `restore` gets what `save` returned
/// last time, possibly from an older devdeck: read it leniently.
pub trait KeepsState: Render + Sized {
    fn save(&self, cx: &App) -> serde_json::Value;
    fn restore(&mut self, state: &serde_json::Value, cx: &mut gpui::Context<Self>);
}

fn focusable<V: Render + Focusable>(view: Entity<V>, cx: &App) -> Built {
    let focus = Some(view.read(cx).focus_handle(cx));
    Built { view: view.into(), focus, save: None, missing: false, live: None }
}

/// A focusable view that keeps its state: restored from `saved`, saved through a weak
/// handle (saving never keeps a closed view alive). Whenever it changes, its deck checks
/// whether its state did (a run started or stopped), to save it soon, not only with the
/// next layout change or at quit.
fn keeping<V: KeepsState + Focusable>(view: Entity<V>, saved: Option<&serde_json::Value>, ctx: &ViewCx, cx: &mut App) -> Built {
    if let Some(state) = saved {
        view.update(cx, |v, cx| v.restore(state, cx));
    }
    let (deck, r) = (ctx.deck.clone(), ctx.view.clone());
    cx.observe(&view, move |_, cx| {
        let _ = deck.update(cx, |d, cx| d.view_changed(&r, cx));
    })
    .detach();
    let weak = view.downgrade();
    let save: Saver = std::rc::Rc::new(move |cx| weak.upgrade().map(|v| v.read(cx).save(cx)));
    Built { save: Some(save), ..focusable(view, cx) }
}

pub fn build(r: &ViewRef, ctx: Option<&ViewCx>, saved: Option<&serde_json::Value>, cx: &mut App) -> Built {
    let addr = ViewAddr::parse(r);
    let message = |text: String, cx: &mut App| Built { view: cx.new(|_| Message(text.into())).into(), focus: None, save: None, missing: true, live: None };
    match (addr, ctx) {
        (ViewAddr::Other(_), ctx) => demo::build(r, ctx, cx),

        (_, None) => Built { missing: false, ..message("This view needs a workspace.".into(), cx) },
        (ViewAddr::App { app }, Some(ctx)) => {
            if ctx.services.read(cx).catalog.app(&app).is_none() {
                return message(format!("{app} isn't an app of this workspace."), cx);
            }
            let v = cx.new(|cx| app_view::AppView::new(app, ctx.clone(), cx));
            keeping(v, saved, ctx, cx)
        }
        (ViewAddr::Terminal { n, dir }, Some(ctx)) => {
            let (runner, env) = {
                let s = ctx.services.read(cx);
                (s.runner.clone(), s.env.clone())
            };
            let key = runner.update(cx, |r, cx| r.open_shell(n, std::path::Path::new(&dir), env, crate::runner::Starter::User, cx));
            let v = cx.new(|cx| run_view::RunView::new(key.app, key.cmd, ctx.clone(), cx));
            keeping(v, saved, ctx, cx)
        }
        (ViewAddr::Run { app, cmd }, Some(ctx)) => {
            let known = ctx.services.read(cx).catalog.app(&app).is_some_and(|a| a.command(&cmd).is_some());
            if !known {
                return message(format!("{app} has no command {cmd}."), cx);
            }
            let v = cx.new(|cx| run_view::RunView::new(app, cmd, ctx.clone(), cx));
            keeping(v, saved, ctx, cx)
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
                keeping(v, saved, ctx, cx)
            }
        },
        (ViewAddr::Pod { env, pod }, Some(ctx)) => match cluster_env(ctx, &env, cx) {
            Err(text) => message(text, cx),
            Ok(cluster) => Built { view: cx.new(|cx| pod_view::PodView::new(env, pod, ctx.clone(), cluster, cx)).into(), focus: None, save: None, missing: false, live: None },
        },
        (ViewAddr::Matrix, Some(ctx)) => match ctx.services.read(cx).cluster.clone() {
            None => message("This workspace has no cluster.".into(), cx),
            Some(cluster) => {
                let v = cx.new(|cx| matrix_view::MatrixView::new(ctx.clone(), cluster, cx));
                focusable(v, cx)
            }
        },
        (ViewAddr::Port { port }, Some(ctx)) => Built { view: cx.new(|cx| port_view::PortView::new(port, ctx.clone(), cx)).into(), focus: None, save: None, missing: false, live: None },
        (ViewAddr::Problems, Some(ctx)) => Built { view: cx.new(|cx| info_view::ProblemsView::new(ctx.clone(), cx)).into(), focus: None, save: None, missing: false, live: None },
        (ViewAddr::Git { root }, Some(ctx)) => {
            let root = std::path::PathBuf::from(root);
            let repo = ctx.services.update(cx, |s, cx| s.repo(&root, cx));
            match repo {
                Some(repo) if repo.read(cx).root == root => {
                    let v = cx.new(|cx| git_view::GitView::new(root, repo, ctx.clone(), cx));
                    focusable(v, cx)
                }
                _ => message(format!("{} isn't a git repository.", devdeck_core::config::shown(&root)), cx),
            }
        }
        (ViewAddr::File { path }, Some(ctx)) => {
            let v = cx.new(|cx| file_view::FileView::new(std::path::PathBuf::from(path), ctx.clone(), cx));
            let built = keeping(v.clone(), saved, ctx, cx);
            live(built, &v, ctx, cx)
        }
        (ViewAddr::Note { id }, Some(ctx)) => {
            if ctx.services.read(cx).notes.read(cx).meta(&id).is_none() {
                return Built { missing: false, ..message("This note was deleted.".into(), cx) };
            }
            let v = cx.new(|cx| note_view::NoteView::new(id, ctx.clone(), cx));
            let built = keeping(v.clone(), saved, ctx, cx);
            live(built, &v, ctx, cx)
        }
        (ViewAddr::Welcome, Some(_)) => Built { view: cx.new(|_| info_view::WelcomeView).into(), focus: None, save: None, missing: false, live: None },
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

/// The tab label of a view from its address; a live view may say better
/// ([`crate::deck::Deck::title`]).
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
        ViewAddr::Note { .. } => "note".into(),
        ViewAddr::File { path } => path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("/").to_string(),
        ViewAddr::Git { root } => format!("{} · git", root.rsplit('/').find(|s| !s.is_empty()).unwrap_or("/")),
        ViewAddr::Terminal { dir, .. } => format!("terminal · {}", terminal_name(&dir)),
        ViewAddr::Other(_) => demo::title(r),
    }
}

/// How a tab names its view (user, 2026-11-09): an icon saying what kind of view it is,
/// then a short name saying which one, like a browser's tab; the full title
/// ([`crate::deck::Deck::title`]) is its tooltip.
#[derive(Clone, Debug, PartialEq)]
pub struct TabLabel {
    pub icon: Icon,
    pub tint: Tint,
    pub name: String,
    /// A second word, dimmer (an env, a command).
    pub detail: Option<String>,
    /// The detail is a protected env: drawn in the error colour.
    pub protected: bool,
}

/// The icon's colour: the tab's own, or a run's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Plain,
    Running,
    Stopping,
    Failed,
}

/// The tab label of a view; `services` (its workspace's) names apps, notes and runs.
pub fn tab_label(r: &ViewRef, services: Option<&Services>, cx: &App) -> TabLabel {
    let last = |p: &str| p.rsplit('/').find(|s| !s.is_empty()).unwrap_or("/").to_string();
    let app_name = |id: &str| services.and_then(|s| s.catalog.app(id)).map_or_else(|| last(id), |a| a.name.clone());
    let protected = |env: &str| services.and_then(|s| s.catalog.env(env)).is_some_and(|e| e.protected);
    let plain = |icon: Icon, name: String| TabLabel { icon, tint: Tint::Plain, name, detail: None, protected: false };
    let in_env = |icon: Icon, name: String, env: String| TabLabel { icon, tint: Tint::Plain, protected: protected(&env), name, detail: Some(env) };
    match ViewAddr::parse(r) {
        ViewAddr::App { app } => plain(Icon::Apps, app_name(&app)),
        ViewAddr::Run { app, cmd } => {
            let state = services.and_then(|s| s.runner.read(cx).state(&crate::runner::RunKey::new(&app, &cmd)));
            let tint = match state {
                Some(RunState::Running) => Tint::Running,
                Some(RunState::Stopping) => Tint::Stopping,
                Some(s) if s.failed().is_some() || s == RunState::NotStarted => Tint::Failed,
                _ => Tint::Plain,
            };
            TabLabel { icon: Icon::Run, tint, name: app_name(&app), detail: Some(devdeck_core::apps::name_of(&cmd).to_string()), protected: false }
        }
        ViewAddr::Svc { env, name } => in_env(Icon::Cluster, name, env),
        ViewAddr::Logs { env, name } => in_env(Icon::Logs, name, env),
        ViewAddr::Pod { env, pod } => in_env(Icon::Pod, pod, env),
        ViewAddr::Matrix => plain(Icon::Matrix, "matrix".into()),
        ViewAddr::Port { port } => plain(Icon::Ports, format!(":{port}")),
        ViewAddr::Problems => plain(Icon::Warning, "problems".into()),
        ViewAddr::Welcome => plain(Icon::Apps, "welcome".into()),
        ViewAddr::Note { id } => plain(Icon::Note, services.and_then(|s| s.notes.read(cx).meta(&id).map(|m| m.name.clone())).unwrap_or_else(|| "note".into())),
        ViewAddr::File { path } => {
            let lang = devdeck_ui::code::language_for(std::path::Path::new(&path));
            plain(if matches!(lang, "text" | "markdown") { Icon::File } else { Icon::FileCode }, last(&path))
        }
        ViewAddr::Git { root } => plain(Icon::Git, last(&root)),
        ViewAddr::Terminal { dir, .. } => plain(Icon::Output, terminal_name(&dir)),
        ViewAddr::Other(_) => plain(Icon::File, demo::title(r)),
    }
}

/// A label's icon colour: a run's state, else `color` (the text's).
pub fn tint_color(l: &TabLabel, color: gpui::Hsla, t: &Theme) -> gpui::Hsla {
    match l.tint {
        Tint::Plain => color,
        Tint::Running => t.ok,
        Tint::Stopping => t.warn,
        Tint::Failed => t.err,
    }
}

/// The icon colours of the run tabs in `layout`: what a window redraws for, rather than
/// for every line a run writes.
pub fn run_tints(layout: &devdeck_core::layout::Layout, services: &Services, cx: &App) -> Vec<(ViewRef, Tint)> {
    layout.views().into_iter().filter(|r| matches!(ViewAddr::parse(r), ViewAddr::Run { .. })).map(|r| (r.clone(), tab_label(&r, Some(services), cx).tint)).collect()
}

/// A tab's icon and short name, the name cut with an ellipsis (R4.4). `color`: the text's.
pub fn tab_label_element(l: &TabLabel, color: gpui::Hsla, t: &Theme) -> gpui::Div {
    let tint = tint_color(l, color, t);
    use gpui::InteractiveElement;
    // The name gives way first; a short detail (an env) stays.
    let name = div().min_w_0().truncate().child(l.name.clone());
    let detail = l.detail.clone().map(|d| div().flex_none().text_color(if l.protected { t.err } else { t.dim }).child(d));
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(t.size(6.))
        .child(icon(l.icon, tint, t))
        .child(div().flex().flex_1().min_w_0().items_center().gap_1().debug_selector(|| "tab-title".into()).child(name).children(detail))
}

/// What a terminal is called wherever it's named: its folder's last part, as the apps
/// list names apps (`entities-graph`, not `~/repos/mf-entities-graph/entities-graph`).
pub fn terminal_name(dir: &str) -> String {
    dir.rsplit('/').find(|s| !s.is_empty()).unwrap_or("/").to_string()
}

/// The mark of a run state (DESIGN visual language): a dot while running, a ring when
/// stopped or never run, turning while stopping, a check for a success, a cross and the
/// code for a failure. `id` is the stopping spinner's; icons are `size` wide.
pub fn run_mark(state: Option<RunState>, id: impl Into<ElementId>, size: Pixels, t: &Theme) -> AnyElement {
    match state {
        Some(RunState::Running) => dot(t.ok, true, t).into_any_element(),
        Some(RunState::Stopping) => spinner(id, t.warn, size, t).into_any_element(),
        Some(RunState::Exited(0)) => icon(Icon::Check, t.ok, t).size(size).into_any_element(),
        Some(s @ (RunState::Exited(_) | RunState::Killed(_) | RunState::NotStarted)) => div()
            .flex()
            .items_center()
            .gap_1()
            .text_color(t.err)
            .child(icon(Icon::Error, t.err, t).size(size))
            .child(s.failed().unwrap_or(1).to_string())
            .into_any_element(),
        Some(RunState::Stopped) | None => dot(t.dim, false, t).into_any_element(),
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

/// An age, short: `45s`, `12m`, `3h`, `9d`.
pub fn fmt_age(s: u64) -> String {
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}
