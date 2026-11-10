//! `Shell`: the workspaces (R2) and the single main window.
//!
//! Each workspace has its own [`Deck`]; they share nothing. One is shown at a time in
//! the main window; the others keep running in the background with their detached
//! windows closed. Quitting from devdeck goes through [`Shell::request_quit`], which
//! asks first when something is running (R8.1); a quit from outside (a signal, the last
//! window closed, on macOS the Dock's Quit) stops everything without asking (R8.2).

use crate::deck::{Deck, NotRevealed, Revealed};
use crate::runner::RunnerEvent;
use crate::services::Services;
use crate::sidebar::Sidebar;
use devdeck_core::view_addr::ViewAddr;
use crate::window_view::{WindowView, window_options};
use devdeck_core::layout::{Layout, MAIN_WINDOW, ViewRef};
use devdeck_core::layout_book::LayoutBook;
use devdeck_core::state::{AppState, Loaded, SidebarState, StateStore};
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{ModalFocus, ToastKind, button, dot, label, modal_frame, pop_modal, push_modal, theme, toasts};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Global, IntoElement, ParentElement, Render, Styled, Subscription,
    Task, WeakFocusHandle, Window, WindowHandle, actions, div, prelude::*,
};
use std::time::Duration;

actions!(devdeck, [ConfirmQuit]);

/// A workspace from its config file, or from the demo.
pub struct WorkspaceSpec {
    pub id: String,
    pub name: String,
    /// The layout used when nothing was saved, or the saved one can't be read.
    pub layout: fn() -> Layout,
    /// `None`: the demo's catalog for `id`.
    pub config: Option<devdeck_core::config::WorkspaceConfig>,
}

impl WorkspaceSpec {
    pub fn new(id: &str, name: &str, layout: fn() -> Layout) -> Self {
        Self { id: id.into(), name: name.into(), layout, config: None }
    }

    /// A configured workspace; it starts with one empty pane.
    pub fn from_config(config: devdeck_core::config::WorkspaceConfig) -> Self {
        Self { id: config.id.clone(), name: config.name.clone(), layout: Layout::new, config: Some(config) }
    }
}

pub struct Workspace {
    pub id: String,
    pub name: String,
    pub deck: Entity<Deck>,
    pub sidebar: Entity<Sidebar>,
    pub services: Entity<Services>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QuitState {
    Idle,
    /// The guard is open.
    Asking,
    /// "Stop all and quit" was chosen; waiting for everything to stop.
    Stopping,
}

/// How long "Stop all" waits for the demo's runs, and at least, before killing what's
/// left and quitting; real processes get their stop sequence's length (see
/// `Runner::longest_stop`) plus [`STOP_MARGIN`].
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_MARGIN: Duration = Duration::from_secs(1);
const STOP_POLL: Duration = Duration::from_millis(50);

pub struct Shell {
    /// Alphabetical by name; ctrl+alt+N follows this order.
    pub workspaces: Vec<Workspace>,
    pub active: usize,
    store: Option<StateStore>,
    main: Option<WindowHandle<WindowView>>,
    /// Main window size in logical pixels, saved in `app.json`.
    pub main_size: Option<(f32, f32)>,
    /// Whether the main window is maximized, saved in `app.json` (R9.8).
    pub main_maximized: bool,
    /// The sidebar's width at UI scale 1, saved in `app.json` (`None`: the theme's).
    pub sidebar_w: Option<f32>,
    pub quit_state: QuitState,
    /// Test seam: the test platform can't quit, so tests turn this off and count.
    pub quit_for_real: bool,
    pub quits: usize,
    /// Everything this devdeck started was killed already (a quit through the shell, a
    /// signal, the last window closed): quitting doesn't do it again.
    pub killed: bool,
    /// Times everything was killed (tests: once per quit, never twice).
    pub kills: usize,
    app_save_failed: bool,
    _stopping: Option<Task<()>>,
    /// Process groups this devdeck started (none for the demo and tests).
    pub log: Option<devdeck_core::process::GroupLog>,
}

struct ShellHandle(Entity<Shell>);
impl Global for ShellHandle {}

pub fn shell(cx: &App) -> Entity<Shell> {
    cx.global::<ShellHandle>().0.clone()
}

pub fn try_shell(cx: &App) -> Option<Entity<Shell>> {
    cx.try_global::<ShellHandle>().map(|h| h.0.clone())
}

impl Shell {
    /// Builds the workspaces (restoring saved layouts when `store` is given), makes the
    /// shell global and opens the main window. Problems reading saved files become toasts.
    pub fn start(mut specs: Vec<WorkspaceSpec>, default_active: &str, store: Option<StateStore>, cx: &mut App) -> Entity<Shell> {
        assert!(!specs.is_empty(), "at least one workspace");
        specs.sort_by_key(|s| s.name.to_lowercase());
        let mut problems = Vec::new();
        let app_state = match store.as_ref().map(StateStore::load_app) {
            Some(Loaded::Ok(s)) => s,
            Some(Loaded::Broken(e)) => {
                problems.push(e);
                AppState::default()
            }
            _ => AppState::default(),
        };
        let find = |id: &str| specs.iter().position(|s| s.id == id);
        let active = app_state.active_workspace.as_deref().and_then(find).or_else(|| find(default_active)).unwrap_or(0);
        let workspaces: Vec<Workspace> = specs
            .into_iter()
            .enumerate()
            .map(|(i, spec)| {
                let layout = match store.as_ref().map(|s| s.load_layout(&spec.id)) {
                    Some(Loaded::Ok(l)) => l,
                    Some(Loaded::Broken(e)) => {
                        problems.push(format!("{e}; using the default {} layout.", spec.name));
                        (spec.layout)()
                    }
                    _ => (spec.layout)(),
                };
                let mut sidebar_state = match store.as_ref().map(|s| s.load_sidebar(&spec.id)) {
                    Some(Loaded::Ok(s)) => s,
                    Some(Loaded::Broken(e)) => {
                        problems.push(e);
                        SidebarState::default()
                    }
                    _ => SidebarState::default(),
                };
                let services = match spec.config.clone() {
                    Some(c) => {
                        let env = crate::environment::environment(cx);
                        Services::from_config(c, sidebar_state.env.clone(), env, store.as_ref().map(|s| s.dir.clone()), cx)
                    }
                    None => Services::demo(&spec.id, sidebar_state.env.clone(), cx),
                };
                let deck_store = store.clone();
                let deck_services = services.clone();
                let book = match store.as_ref().map(|s| s.load_book(&spec.id)) {
                    Some(Loaded::Ok(b)) => b,
                    Some(Loaded::Broken(e)) => {
                        problems.push(e);
                        LayoutBook::default()
                    }
                    _ => LayoutBook::default(),
                };
                if let Some(pins) = sidebar_state.pins.get_mut("layouts") {
                    pin_layouts_by_id(pins, &book);
                }
                let views = match store.as_ref().map(|s| s.load_views(&spec.id)) {
                    Some(Loaded::Ok(v)) => v,
                    Some(Loaded::Broken(e)) => {
                        problems.push(e);
                        Default::default()
                    }
                    _ => Default::default(),
                };
                let deck = cx.new(|cx| {
                    let mut deck =
                        Deck::new(layout).in_workspace(spec.id.clone(), deck_store, i == active).with_services(deck_services).with_book(book).with_view_states(views);
                    deck.follow_services(cx);
                    deck
                });
                let (ws, sidebar_deck, sidebar_store) = (spec.id.clone(), deck.clone(), store.clone());
                let sidebar_services = services.clone();
                let sidebar = cx.new(|cx| Sidebar::new(ws, sidebar_services, sidebar_deck, sidebar_store, sidebar_state, cx));
                Workspace { id: spec.id, name: spec.name, deck, sidebar, services }
            })
            .collect();
        let decks: Vec<Entity<Deck>> = workspaces.iter().map(|w| w.deck.clone()).collect();
        for w in &workspaces {
            watch_results(w, cx);
        }
        let shell = cx.new(|_| Shell {
            workspaces,
            active,
            store,
            main: None,
            main_size: app_state.main_size,
            main_maximized: app_state.main_maximized,
            sidebar_w: app_state.sidebar_w,
            quit_state: QuitState::Idle,
            quit_for_real: true,
            quits: 0,
            killed: false,
            kills: 0,
            app_save_failed: false,
            _stopping: None,
            log: None,
        });
        cx.set_global(ShellHandle(shell.clone()));
        // A quit that didn't come through the shell, a signal or the last window closing
        // (on macOS: the Dock's Quit, logout, shutdown) still stops everything (R8.2).
        let quitting = shell.downgrade();
        cx.on_app_quit(move |cx| {
            if let Some(s) = quitting.upgrade() {
                s.update(cx, |s, cx| {
                    if s.quit_for_real && !s.killed {
                        s.kill_to_quit(cx);
                    }
                });
            }
            async {}
        })
        .detach();
        // Every workspace's views start now (they keep running in the background); only
        // the active one opens its detached windows.
        for d in decks {
            d.update(cx, |d, cx| d.sync(cx));
        }
        let size = shell.read(cx).main_size.unwrap_or((1600., 1000.));
        let view_shell = shell.clone();
        match cx.open_window(window_options(size, shell.read(cx).main_maximized, cx), |window, cx| cx.new(|cx| WindowView::main(view_shell, window, cx))) {
            Ok(h) => shell.update(cx, |s, _| s.main = Some(h)),
            Err(e) => problems.push(format!("could not open the main window: {e}")),
        }
        for p in problems {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, p, vec![], cx));
        }
        report_problems(&shell, cx);
        shell
    }

    /// One unsaved workspace with this layout, quitting disabled (tests).
    pub fn start_single(layout: fn() -> Layout, cx: &mut App) -> Entity<Shell> {
        let shell = Self::start(vec![WorkspaceSpec::new("work", "work", layout)], "work", None, cx);
        shell.update(cx, |s, _| s.quit_for_real = false);
        shell
    }

    pub fn active_deck(&self) -> Entity<Deck> {
        self.workspaces[self.active].deck.clone()
    }

    pub fn active_sidebar(&self) -> Entity<Sidebar> {
        self.workspaces[self.active].sidebar.clone()
    }

    pub fn active_name(&self) -> &str {
        &self.workspaces[self.active].name
    }

    pub fn main_window(&self) -> Option<WindowHandle<WindowView>> {
        self.main
    }

    /// Shows another workspace. The old one keeps running with its detached windows
    /// closed; the new one's detached windows open, then the main window comes back
    /// to the front.
    pub fn switch(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.workspaces.len() || index == self.active {
            return;
        }
        let old = self.active_deck();
        self.active = index;
        let new = self.active_deck();
        old.update(cx, |d, cx| d.set_active(false, cx));
        new.update(cx, |d, cx| d.set_active(true, cx));
        self.save_app(cx);
        if let Some(main) = self.main {
            cx.defer(move |cx| {
                let _ = main.update(cx, |_, window, _| window.activate_window());
            });
        }
        cx.notify();
    }

    /// Brings `view` of workspace `workspace` (an id) on screen, the one way a
    /// notification's Open, the palette or the API shows a view where it is (R2.2, R3.8):
    /// the workspace shown, then the view found in its live layout or a saved one
    /// ([`Deck::reveal`]), its window brought forward (where the window system refuses,
    /// it asks for attention) with the keys in the view, its tab flashing. Open nowhere,
    /// it opens in the focused pane ("Open here"). Nothing changes when the workspace is
    /// gone, the view's target is gone (logged), or a dialog is open (it keeps the keys);
    /// the caller says it.
    pub fn reveal(&mut self, workspace: &str, view: &ViewRef, cx: &mut Context<Self>) -> Result<Revealed, NotRevealed> {
        let Some(i) = self.workspaces.iter().position(|w| w.id == workspace) else {
            log::warn!("reveal {}: no workspace {workspace}", view.0);
            return Err(NotRevealed::WorkspaceGone);
        };
        if devdeck_ui::overlay::any_modal(cx) {
            return Err(NotRevealed::Modal);
        }
        let deck = self.workspaces[i].deck.clone();
        if let Some(why) = deck.read(cx).gone(view, cx) {
            log::warn!("reveal {}: {why}", view.0);
            return Err(NotRevealed::ViewGone(why));
        }
        self.switch(i, cx);
        let (revealed, window) = deck.update(cx, |d, cx| d.reveal(view, cx))?;
        let main = self.main;
        // After the switch's and the layout's deferred work (windows open, the main window
        // follows the workspace), and outside any window's update.
        cx.defer(move |cx| {
            let handle = if window == MAIN_WINDOW { main } else { deck.read(cx).os_window(window) };
            if let Some(h) = handle {
                let _ = h.update(cx, |v, window, cx| v.bring_forward(window, cx));
            }
        });
        Ok(revealed)
    }

    /// Whether `view` is open in workspace `workspace` (its live layout or a saved one).
    pub fn holds(&self, workspace: &str, view: &ViewRef, cx: &App) -> bool {
        self.workspaces.iter().find(|w| w.id == workspace).is_some_and(|w| w.deck.read(cx).holds(view))
    }

    /// What runs, per workspace (workspaces with nothing running are left out).
    pub fn running(&self, cx: &App) -> Vec<(String, Vec<String>)> {
        self.workspaces
            .iter()
            .map(|w| (w.name.clone(), w.services.read(cx).runner.read(cx).running()))
            .filter(|(_, r)| !r.is_empty())
            .collect()
    }

    /// The one way to quit: at once when nothing runs, otherwise through the guard.
    /// Asking again while the guard is open does nothing.
    pub fn request_quit(&mut self, cx: &mut Context<Self>) {
        if self.quit_state != QuitState::Idle {
            return;
        }
        let running = self.running(cx);
        let Some(main) = self.main.filter(|_| !running.is_empty()) else {
            // Nothing to ask about; idle shells still get their hang-up (their history).
            let alive = self.workspaces.iter().any(|w| w.services.read(cx).runner.read(cx).alive());
            if alive { self.stop_all_and_quit(cx) } else { self.quit_now(cx) }
            return;
        };
        self.quit_state = QuitState::Asking;
        let shell = cx.entity();
        cx.defer(move |cx| {
            let _ = main.update(cx, |_, window, cx| {
                window.activate_window();
                open_quit_guard(shell, running, window, cx);
            });
        });
        cx.notify();
    }

    pub fn cancel_quit(&mut self, cx: &mut Context<Self>) {
        if self.quit_state == QuitState::Asking {
            self.quit_state = QuitState::Idle;
            cx.notify();
        }
    }

    /// Stops everything in every workspace, waits until it stopped (or the timeout),
    /// then quits.
    pub fn stop_all_and_quit(&mut self, cx: &mut Context<Self>) {
        if self.quit_state == QuitState::Stopping {
            return;
        }
        self.quit_state = QuitState::Stopping;
        for w in &self.workspaces {
            let runner = w.services.read(cx).runner.clone();
            runner.update(cx, |r, cx| r.stop_all(cx));
        }
        cx.notify();
        let longest = self.workspaces.iter().map(|w| w.services.read(cx).runner.read(cx).longest_stop()).max().unwrap_or_default();
        let timeout = STOP_TIMEOUT.max(longest + STOP_MARGIN);
        self._stopping = Some(cx.spawn(async move |this, cx| {
            let mut waited = Duration::ZERO;
            loop {
                // Idle shells too: they get their hang-up's time.
                let done = this.update(cx, |s, cx| !s.workspaces.iter().any(|w| w.services.read(cx).runner.read(cx).alive())).unwrap_or(true);
                if done || waited >= timeout {
                    break;
                }
                cx.background_executor().timer(STOP_POLL).await;
                waited += STOP_POLL;
            }
            let _ = this.update(cx, |s, cx| {
                s.kill_everything(cx);
                s.quit_now(cx);
            });
        }));
    }

    /// Quitting at once (a signal): what runs counts as stopped to quit, so it starts
    /// again on reopen; everything is killed, and saved.
    pub fn kill_to_quit(&mut self, cx: &mut Context<Self>) {
        for w in &self.workspaces {
            let runner = w.services.read(cx).runner.clone();
            runner.update(cx, |r, _| r.quitting());
        }
        self.kill_everything(cx);
        self.flush(cx);
    }

    /// Kills, now, every process this devdeck started that may still run: the runs' and
    /// what the group log recorded (R8.2).
    pub fn kill_everything(&mut self, cx: &App) {
        self.killed = true;
        self.kills += 1;
        let mut pids: Vec<u32> = self.workspaces.iter().flat_map(|w| w.services.read(cx).runner.read(cx).pids()).map(|(_, p)| p).collect();
        pids.extend(self.log.iter().flat_map(|l| l.live()));
        pids.sort_unstable();
        pids.dedup();
        if !pids.is_empty() {
            devdeck_core::process::kill_all(&pids);
        }
        // What the runs left in their scopes (a build's daemon) goes with devdeck.
        if let Some(log) = &self.log {
            log.end_ours();
        }
    }

    /// Saves everything pending and quits.
    fn quit_now(&mut self, cx: &mut Context<Self>) {
        self.flush(cx);
        self.quits += 1;
        if self.quit_for_real {
            // Nothing was running, or it stopped: what the runs left in their scopes (a
            // build's daemon) still goes with devdeck.
            if !self.killed {
                self.kill_everything(cx);
            }
            cx.quit();
        } else if self.quit_state != QuitState::Idle {
            // Tests: close the guard as quitting would.
            self.quit_state = QuitState::Idle;
            self._stopping = None;
            if let Some(main) = self.main {
                cx.defer(move |cx| {
                    let _ = main.update(cx, |_, window, cx| pop_modal(window, cx));
                });
            }
            cx.notify();
        }
    }

    /// Writes pending layouts and the app state now.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        for w in &self.workspaces {
            w.deck.update(cx, |d, cx| d.flush(cx));
            let notes = w.services.read(cx).notes.clone();
            notes.update(cx, |n, cx| n.flush(cx));
        }
        self.save_app(cx);
    }

    fn save_app(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &self.store else { return };
        let state = AppState { active_workspace: Some(self.workspaces[self.active].id.clone()), main_size: self.main_size, main_maximized: self.main_maximized, sidebar_w: self.sidebar_w };
        match store.save_app(&state) {
            Ok(()) => self.app_save_failed = false,
            Err(e) if !self.app_save_failed => {
                self.app_save_failed = true;
                toasts(cx).update(cx, |t, cx| t.push(ToastKind::Err, format!("Could not save app state: {e}"), vec![], cx));
            }
            Err(_) => {}
        }
    }
}

/// What config files, discovery and overrides reported, as one toast with a count; View
/// opens the active workspace's problems view (the list stays there, live).
pub fn report_problems(shell: &Entity<Shell>, cx: &mut App) {
    let machine = crate::environment::environment(cx).read(cx).problems().len();
    let s = shell.read(cx);
    let count = machine + s.workspaces.iter().map(|w| w.services.read(cx).all_problems(cx).len()).sum::<usize>();
    if count == 0 {
        return;
    }
    let text = if count == 1 { "1 problem reading the config or the projects".to_string() } else { format!("{count} problems reading the config or the projects") };
    let toast = devdeck_ui::NewToast::new(ToastKind::Warn, text).note(crate::notifications::problems_action("View"));
    toasts(cx).update(cx, |t, cx| t.push_new(toast, cx));
}

/// Layout pins saved before layouts had ids hold names: each moves to its layout's id.
fn pin_layouts_by_id(pins: &mut std::collections::BTreeSet<String>, book: &LayoutBook) {
    let named: Vec<(String, String)> =
        pins.iter().filter(|k| !book.layouts.contains_key(*k)).filter_map(|k| Some((k.clone(), book.id_named(k)?))).collect();
    for (name, id) in named {
        pins.remove(&name);
        pins.insert(id);
    }
}

/// A run ending on its own in any workspace becomes a toast naming the workspace, kept
/// as that workspace's notification: an error when it failed, a success with how long it
/// took; Open (or "Open here") reveals the run's tab in that workspace.
fn watch_results(w: &Workspace, cx: &mut App) {
    let runner = w.services.read(cx).runner.clone();
    let (name, id) = (w.name.clone(), w.id.clone());
    cx.subscribe(&runner, move |runner, event: &RunnerEvent, cx| {
        let (key, kind, outcome) = match event {
            RunnerEvent::Failed { key, code } => (key, ToastKind::Err, format!("exited {code}")),
            RunnerEvent::Succeeded { key, took } => (key, ToastKind::Ok, format!("succeeded in {}", devdeck_core::runs::took(*took))),
            RunnerEvent::Output { .. } => return,
        };
        let addr = ViewAddr::Run { app: key.app.clone(), cmd: key.cmd.clone() };
        let text = format!("{name} · {} {outcome}", runner.read(cx).title(key));
        let open = crate::notifications::reveal_action(&id, &addr.to_ref(), cx);
        let toast = devdeck_ui::NewToast::new(kind, text).source(Some(id.clone().into())).note(open);
        toasts(cx).update(cx, |t, cx| t.push_new(toast, cx));
    })
    .detach();
}

// ------------------------------------------------------------------ app-level entry points

/// Quit from anywhere (ctrl+q in any window, title-bar ×, window manager close).
pub fn request_quit(cx: &mut App) {
    match try_shell(cx) {
        Some(s) => s.update(cx, |s, cx| s.request_quit(cx)),
        None => cx.quit(),
    }
}

pub fn switch(index: usize, cx: &mut App) {
    if let Some(s) = try_shell(cx) {
        s.update(cx, |s, cx| s.switch(index, cx));
    }
}

// ------------------------------------------------------------------ quit guard

fn open_quit_guard(shell: Entity<Shell>, running: Vec<(String, Vec<String>)>, window: &mut Window, cx: &mut App) {
    let restore = window.focused(cx).map(|f| f.downgrade());
    let guard = cx.new(|cx| QuitGuard::new(shell, running, restore, cx));
    push_modal(guard.clone().into(), window, cx);
    let first = guard.read(cx).focus.0[0].clone();
    first.focus(window, cx);
}

/// "Quit Kugelblitz?" with what runs, per workspace. Enter picks the focused button
/// ("Stop all and quit" first), Esc cancels.
pub struct QuitGuard {
    shell: Entity<Shell>,
    running: Vec<(String, Vec<String>)>,
    pub focus: ModalFocus,
    restore: Option<WeakFocusHandle>,
    _observe: Subscription,
}

impl QuitGuard {
    fn new(shell: Entity<Shell>, running: Vec<(String, Vec<String>)>, restore: Option<WeakFocusHandle>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&shell, |_, _, cx| cx.notify());
        Self { shell, running, focus: ModalFocus(vec![cx.focus_handle(), cx.focus_handle()]), restore, _observe: observe }
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shell.read(cx).quit_state != QuitState::Asking {
            return;
        }
        pop_modal(window, cx);
        self.shell.update(cx, |s, cx| s.cancel_quit(cx));
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            h.focus(window, cx);
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.shell.update(cx, |s, cx| s.stop_all_and_quit(cx));
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus.0[1].is_focused(window) {
            self.cancel(window, cx);
        } else {
            self.stop(cx);
        }
    }
}

pub(crate) fn focus_button(id: &'static str, text: &'static str, handle: &FocusHandle, t: &devdeck_ui::Theme) -> gpui::Stateful<gpui::Div> {
    let ring = t.focus;
    button(id, text, t).border_1().border_color(t.line).track_focus(handle).focus(move |s| s.border_color(ring))
}

impl Render for QuitGuard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let stopping = self.shell.read(cx).quit_state == QuitState::Stopping;
        let mut list = div().flex().flex_col().gap_1();
        for (ws, items) in &self.running {
            list = list.child(label(ws.clone()).text_color(t.hi));
            for item in items {
                list = list.child(div().pl_2().flex().items_center().gap_1().child(dot(t.warn, true, &t)).child(label(item.clone()).text_color(t.warn)));
            }
        }
        let buttons = if stopping {
            div().flex().justify_end().child(label("Stopping…").text_color(t.mute))
        } else {
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(focus_button("quit-cancel", "Cancel", &self.focus.0[1], &t).on_click(cx.listener(|this, _, w, cx| this.cancel(w, cx))))
                .child(
                    focus_button("quit-stop", "Stop all and quit", &self.focus.0[0], &t)
                        .debug_selector(|| "quit-stop".into())
                        .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                )
        };
        let content = div()
            .key_context("Modal QuitGuard")
            .flex()
            .flex_col()
            .gap_2()
            .on_action(cx.listener(|this, _: &ModalNext, window, cx| this.focus.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalPrev, window, cx| this.focus.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ModalClose, window, cx| this.cancel(window, cx)))
            .on_action(cx.listener(|this, _: &ConfirmQuit, window, cx| this.confirm(window, cx)))
            .child(div().text_color(t.hi).child("Quit Kugelblitz?"))
            .child(label("These are still running:").text_color(t.mute))
            .child(list)
            .child(buttons);
        modal_frame(content.into_any_element(), window, &t)
    }
}
