//! `Shell`: the workspaces (R2) and the single main window.
//!
//! Each workspace has its own [`Deck`]; they share nothing. One is shown at a time in
//! the main window; the others keep running in the background with their detached
//! windows closed. Every way of quitting goes through [`Shell::request_quit`], which
//! asks first when something is running (R8.1).

use crate::deck::Deck;
use crate::runner::RunnerEvent;
use crate::services::Services;
use crate::sidebar::Sidebar;
use crate::views::ViewCx;
use devdeck_core::view_addr::ViewAddr;
use crate::window_view::{WindowView, window_options};
use devdeck_core::layout::Layout;
use devdeck_core::state::{AppState, Loaded, SidebarState, StateStore};
use devdeck_ui::overlay::{ModalClose, ModalNext, ModalPrev};
use devdeck_ui::{ModalFocus, ToastKind, button, label, modal_frame, pop_modal, push_modal, theme, toasts};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Global, IntoElement, ParentElement, Render, Styled, Subscription,
    Task, WeakFocusHandle, Window, WindowHandle, actions, div, prelude::*,
};
use std::time::Duration;

actions!(devdeck, [ConfirmQuit]);

/// A workspace as defined by configuration (M2) or the demo (M1).
pub struct WorkspaceSpec {
    pub id: String,
    pub name: String,
    /// The layout used when nothing was saved, or the saved one can't be read.
    pub layout: fn() -> Layout,
}

impl WorkspaceSpec {
    pub fn new(id: &str, name: &str, layout: fn() -> Layout) -> Self {
        Self { id: id.into(), name: name.into(), layout }
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

/// How long "Stop all" waits before quitting anyway.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_POLL: Duration = Duration::from_millis(50);

pub struct Shell {
    /// Alphabetical by name; ctrl+alt+N follows this order.
    pub workspaces: Vec<Workspace>,
    pub active: usize,
    store: Option<StateStore>,
    main: Option<WindowHandle<WindowView>>,
    /// Main window size in logical pixels, saved in `app.json`.
    pub main_size: Option<(f32, f32)>,
    pub quit_state: QuitState,
    /// Test seam: the test platform can't quit, so tests turn this off and count.
    pub quit_for_real: bool,
    pub quits: usize,
    app_save_failed: bool,
    _stopping: Option<Task<()>>,
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
                let sidebar_state = match store.as_ref().map(|s| s.load_sidebar(&spec.id)) {
                    Some(Loaded::Ok(s)) => s,
                    Some(Loaded::Broken(e)) => {
                        problems.push(e);
                        SidebarState::default()
                    }
                    _ => SidebarState::default(),
                };
                let services = Services::demo(&spec.id, &spec.name, sidebar_state.env.clone(), cx);
                let deck_store = store.clone();
                let deck_services = services.clone();
                let deck = cx.new(|cx| {
                    let ctx = ViewCx { services: deck_services, deck: cx.weak_entity() };
                    Deck::new(layout).in_workspace(spec.id.clone(), deck_store, i == active).with_ctx(ctx)
                });
                let (ws, sidebar_deck, sidebar_store) = (spec.id.clone(), deck.clone(), store.clone());
                let sidebar_services = services.clone();
                let sidebar = cx.new(|cx| Sidebar::new(ws, sidebar_services, sidebar_deck, sidebar_store, sidebar_state, cx));
                Workspace { id: spec.id, name: spec.name, deck, sidebar, services }
            })
            .collect();
        let decks: Vec<Entity<Deck>> = workspaces.iter().map(|w| w.deck.clone()).collect();
        for (i, w) in workspaces.iter().enumerate() {
            watch_failures(i, w, cx);
        }
        let shell = cx.new(|_| Shell {
            workspaces,
            active,
            store,
            main: None,
            main_size: app_state.main_size,
            quit_state: QuitState::Idle,
            quit_for_real: true,
            quits: 0,
            app_save_failed: false,
            _stopping: None,
        });
        cx.set_global(ShellHandle(shell.clone()));
        // Every workspace's views start now (they keep running in the background); only
        // the active one opens its detached windows.
        for d in decks {
            d.update(cx, |d, cx| d.sync(cx));
        }
        let size = shell.read(cx).main_size.unwrap_or((1600., 1000.));
        let view_shell = shell.clone();
        match cx.open_window(window_options(size, cx), |window, cx| cx.new(|cx| WindowView::main(view_shell, window, cx))) {
            Ok(h) => shell.update(cx, |s, _| s.main = Some(h)),
            Err(e) => problems.push(format!("could not open the main window: {e}")),
        }
        for p in problems {
            toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, p, vec![], cx));
        }
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
            self.quit_now(cx);
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
        self._stopping = Some(cx.spawn(async move |this, cx| {
            let mut waited = Duration::ZERO;
            loop {
                let done = this.update(cx, |s, cx| s.running(cx).is_empty()).unwrap_or(true);
                if done || waited >= STOP_TIMEOUT {
                    break;
                }
                cx.background_executor().timer(STOP_POLL).await;
                waited += STOP_POLL;
            }
            let _ = this.update(cx, |s, cx| s.quit_now(cx));
        }));
    }

    /// Saves everything pending and quits.
    fn quit_now(&mut self, cx: &mut Context<Self>) {
        self.flush(cx);
        self.quits += 1;
        if self.quit_for_real {
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
        }
        self.save_app(cx);
    }

    fn save_app(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &self.store else { return };
        let state = AppState { active_workspace: Some(self.workspaces[self.active].id.clone()), main_size: self.main_size };
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

/// A run failing in any workspace becomes an error toast naming the workspace; "View
/// output" shows that workspace and opens the run.
fn watch_failures(index: usize, w: &Workspace, cx: &mut App) {
    let runner = w.services.read(cx).runner.clone();
    let (name, deck) = (w.name.clone(), w.deck.downgrade());
    let services = w.services.clone();
    cx.subscribe(&runner, move |_, event: &RunnerEvent, cx| {
        let RunnerEvent::Failed { key, code } = event;
        let addr = ViewAddr::Run { app: key.app.clone(), cmd: key.cmd.clone() };
        let ctx = ViewCx { services: services.clone(), deck: deck.clone() };
        let view: devdeck_ui::overlay::MenuAction = std::rc::Rc::new(move |_, cx| {
            switch(index, cx);
            ctx.open(addr.clone(), cx);
        });
        let text = format!("{name} · {} exited {code}", key.title());
        toasts(cx).update(cx, |t, cx| t.push(ToastKind::Err, text, vec![("View output".into(), view)], cx));
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

/// "Quit devdeck?" with what runs, per workspace. Enter picks the focused button
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

fn focus_button(id: &'static str, text: &'static str, handle: &FocusHandle, t: &devdeck_ui::Theme) -> gpui::Stateful<gpui::Div> {
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
                list = list.child(label(format!("  ● {item}")).text_color(t.warn));
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
            .child(div().text_color(t.hi).child("Quit devdeck?"))
            .child(label("These are still running:").text_color(t.mute))
            .child(list)
            .child(buttons)
            .child(label("enter picks · tab moves · esc cancels").text_color(t.mute));
        modal_frame(content.into_any_element(), window, &t)
    }
}
