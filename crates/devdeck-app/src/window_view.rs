//! Root view of one OS window: title bar, pane tree, window edges.
//! The main window shows the active workspace's `Deck` and switches with it; a
//! detached window always belongs to the deck that opened it.
//!
//! Rendering works on a snapshot (`Frame`) taken from the deck at the start of
//! `render`, so drawing never holds a borrow of the deck.

use crate::actions::*;
use crate::deck::{Deck, SendSource};
use crate::stats;
use crate::app_settings;
use crate::settings_view::SettingsView;
use crate::shell::{self, Shell};
use crate::sidebar::{self, Activity, DraggedListItem, Sidebar, SidebarEscape};
use crate::views;
use devdeck_ui::{DragGhost, Empty, MenuItem, Theme, apply_window_scale, button, icon_button, label, logical, open_menu, theme, tip};
use devdeck_core::layout::{Axis, Direction, Drop, Edge, Layout, MAIN_WINDOW, Node, PaneId, SplitId, ViewRef, WindowId};
use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, Context, CursorStyle, Decorations, DragMoveEvent, Entity, EntityId,
    FocusHandle, Focusable, HitboxBehavior, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Point, Render, ResizeEdge, SharedString, StyleRefinement, Size, StatefulInteractiveElement, Styled, Subscription, WeakEntity,
    Tiling, WeakFocusHandle, Window, WindowBounds, WindowDecorations, WindowOptions, canvas, div, point, prelude::*, relative,
    size,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

/// Drag payloads carry their deck: a drop that lands in another workspace's tree
/// (after a switch mid-drag) is ignored.
#[derive(Clone)]
pub struct DraggedTab {
    pub deck: EntityId,
    pub pane: PaneId,
    pub index: usize,
    pub title: SharedString,
}

#[derive(Clone)]
pub struct DraggedPane {
    pub deck: EntityId,
    pub pane: PaneId,
    pub title: SharedString,
}

#[derive(Clone)]
pub struct DraggedDivider {
    pub deck: EntityId,
    pub split: SplitId,
    pub index: usize,
    pub axis: Axis,
}

pub struct WindowView {
    id: WindowId,
    /// The shell for the main window; `None` for a detached one.
    shell: Option<WeakEntity<Shell>>,
    deck: Entity<Deck>,
    /// Root focus: global keys (scale, settings, quit).
    focus: FocusHandle,
    /// Pane area focus: pane keys only work while it (or something in it) has focus.
    panes_focus: FocusHandle,
    settings: Option<Entity<SettingsView>>,
    /// Main window: where focus was in each workspace, restored on switching back.
    remembered: HashMap<EntityId, WeakFocusHandle>,
    /// While a tab is dragged over a tab strip: the pane and the place it would land at.
    tab_insert: Rc<Cell<Option<(PaneId, usize)>>>,
    /// Where focus was when Settings opened.
    before_settings: Option<WeakFocusHandle>,
    deck_sub: Subscription,
    /// Main window: the active workspace's sidebar.
    sidebar: Option<(Entity<Sidebar>, Subscription)>,
    /// Top-left of the pane area (keyboard-opened menus are placed from it).
    panes_origin: Rc<Cell<Point<Pixels>>>,
    /// Focused pane and its active view when keys last moved to a view.
    view_target: Option<(PaneId, Option<devdeck_core::layout::ViewRef>)>,
    _observe: Vec<Subscription>,
}

/// Options shared by every devdeck window; `size` in logical pixels.
pub fn window_options((w, h): (f32, f32), cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(logical(w), logical(h)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("devdeck".into()),
        window_min_size: Some(size(logical(480.), logical(320.))),
        ..Default::default()
    }
}

/// The window's size to remember, or `None` while the compositor holds it to the screen
/// (maximized, fullscreen, snapped): that size isn't the one to reopen with, and the
/// last free size stays.
fn free_size(window: &Window) -> Option<(f32, f32)> {
    if tiling(window) != Tiling::default() {
        return None;
    }
    let s = window.bounds().size;
    Some((f32::from(s.width), f32::from(s.height)))
}

/// The sides the compositor holds against the screen. GPUI reports maximized and
/// fullscreen as every side; with server decorations it reports none, so those two are
/// asked directly.
fn tiling(window: &Window) -> Tiling {
    match window.window_decorations() {
        Decorations::Client { tiling } => tiling,
        Decorations::Server if window.is_maximized() || window.is_fullscreen() => Tiling::tiled(),
        Decorations::Server => Tiling::default(),
    }
}

/// The 1px frame a window draws itself (client-side decorations), so it stands out from
/// a background of its own colour: brighter while the window is active; none when
/// maximized or fullscreen (the screen is its frame), nor when the compositor draws one.
fn frame_color(decorations: Decorations, maximized: bool, fullscreen: bool, active: bool, t: &Theme) -> Option<gpui::Hsla> {
    let ours = matches!(decorations, Decorations::Client { .. });
    (ours && !maximized && !fullscreen).then_some(if active { t.mute } else { t.line })
}

/// The workspace menu (title-bar button): alphabetical, current one checked.
pub fn workspace_menu_items(shell: &Entity<Shell>, cx: &App) -> Vec<MenuItem> {
    let s = shell.read(cx);
    s.workspaces
        .iter()
        .enumerate()
        .map(|(i, w)| {
            MenuItem::new(w.name.clone(), move |_, cx| shell::switch(i, cx)).checked(i == s.active)
        })
        .collect()
}

/// Everything a render pass needs, copied out of the deck.
struct Frame<'a> {
    layout: &'a Layout,
    /// Send to badges: pane → (number, can take the pick).
    numbers: &'a HashMap<PaneId, (usize, bool)>,
    /// Where Esc in an input of a pane's view sends focus (the view, else the panes).
    view_focus: &'a HashMap<PaneId, FocusHandle>,
    views: &'a HashMap<PaneId, AnyView>,
    window: WindowId,
    deck: &'a Entity<Deck>,
    t: &'a Theme,
    dragging: bool,
    tab_insert: &'a Rc<Cell<Option<(PaneId, usize)>>>,
}

impl WindowView {
    /// The main window: shows the shell's active workspace. Closing it asks to quit.
    pub fn main(shell: Entity<Shell>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let deck = shell.read(cx).active_deck();
        let mut observe = vec![
            // Settings cover the panes: their views are off screen meanwhile.
            cx.observe(&app_settings::settings(cx), |this, _, cx| {
                let open = app_settings::is_open(cx);
                this.deck.update(cx, |d, cx| d.set_covered(open, cx));
                cx.notify();
            }),
            cx.observe_in(&shell, window, |this, shell, window, cx| this.follow_shell(&shell, window, cx)),
        ];
        let size_shell = shell.downgrade();
        observe.push(cx.observe_window_bounds(window, move |_, window, cx| {
            if let Some(size) = free_size(window) {
                let _ = size_shell.update(cx, |s, _| s.main_size = Some(size));
            }
        }));
        window.on_window_should_close(cx, |_, cx| {
            cx.defer(shell::request_quit);
            false
        });
        window.set_window_title(&format!("devdeck · {}", shell.read(cx).active_name()));
        let sidebar = shell.read(cx).active_sidebar();
        let mut this = Self::build(MAIN_WINDOW, Some(shell.downgrade()), deck, observe, window, cx);
        this.sidebar = Some((sidebar.clone(), cx.observe(&sidebar, |_, _, cx| cx.notify())));
        this
    }

    /// A detached window of `deck`. Closing it closes its tabs.
    pub fn detached(id: WindowId, deck: Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut observe = vec![cx.observe(&app_settings::settings(cx), |_, _, cx| cx.notify())];
        let size_deck = deck.downgrade();
        observe.push(cx.observe_window_bounds(window, move |_, window, cx| {
            if let Some(size) = free_size(window) {
                let _ = size_deck.update(cx, |d, cx| d.record_size(id, size, cx));
            }
        }));
        let close_deck = deck.clone();
        window.on_window_should_close(cx, move |_, cx| {
            close_deck.update(cx, |d, cx| d.os_window_closed(id, cx));
            true
        });
        window.set_window_title(&format!("devdeck · {} · detached", deck.read(cx).workspace));
        Self::build(id, None, deck, observe, window, cx)
    }

    fn build(
        id: WindowId,
        shell: Option<WeakEntity<Shell>>,
        deck: Entity<Deck>,
        mut observe: Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The frame is brighter while the window is active.
        observe.push(cx.observe_window_activation(window, |_, _, cx| cx.notify()));
        let deck_sub = Self::observe_deck(&deck, window, cx);
        let focus = cx.focus_handle();
        let panes_focus = cx.focus_handle();
        panes_focus.focus(window, cx);
        let mut this = Self {
            id,
            shell,
            deck,
            focus,
            panes_focus,
            settings: None,
            remembered: HashMap::new(),
            tab_insert: Rc::default(),
            before_settings: None,
            deck_sub,
            sidebar: None,
            view_target: None,
            panes_origin: Rc::new(Cell::new(Point::default())),
            _observe: observe,
        };
        this.sync_view_focus(window, cx);
        this
    }

    fn observe_deck(deck: &Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Subscription {
        cx.observe_in(deck, window, |this, _, window, cx| {
            this.sync_view_focus(window, cx);
            cx.notify();
        })
    }

    /// When the focused pane or its active tab changed while focus is in the pane area,
    /// gives focus to that view (if it takes keys) so its keys work without a click.
    fn sync_view_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.focused_pane(cx).map(|p| (p, self.deck.read(cx).layout().pane(p).ok().and_then(|p| p.active_view().cloned())));
        if target == self.view_target {
            return;
        }
        self.view_target = target.clone();
        if target.is_none() {
            return;
        }
        // Only when focus is in the pane area, or not in this window at all (its view
        // closed or moved to another window); never away from the sidebar or Settings.
        if !self.panes_focus.contains_focused(window, cx) && self.focus.contains_focused(window, cx) {
            return;
        }
        self.focus_panes(window, cx);
    }

    /// Gives the keys to the pane area: the focused pane's view when it takes keys (so its
    /// own keys work), else the pane area itself. Every "back to the panes" goes here.
    fn focus_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.focused_pane(cx);
        match pane.and_then(|p| self.deck.read(cx).view_focus(p)) {
            Some(h) => h.focus(window, cx),
            None => self.panes_focus.focus(window, cx),
        }
    }

    fn is_main(&self) -> bool {
        self.shell.is_some()
    }

    /// Main window: when the active workspace changed, show its deck and put focus
    /// back where it was in that workspace.
    fn follow_shell(&mut self, shell: &Entity<Shell>, window: &mut Window, cx: &mut Context<Self>) {
        let deck = shell.read(cx).active_deck();
        if deck != self.deck {
            if !app_settings::is_open(cx)
                && let Some(f) = window.focused(cx)
            {
                self.remembered.insert(self.deck.entity_id(), f.downgrade());
            }
            self.deck_sub = Self::observe_deck(&deck, window, cx);
            let open = app_settings::is_open(cx);
            deck.update(cx, |d, cx| d.set_covered(open, cx));
            self.deck = deck;
            self.view_target = None;
            let sidebar = shell.read(cx).active_sidebar();
            self.sidebar = Some((sidebar.clone(), cx.observe(&sidebar, |_, _, cx| cx.notify())));
            window.set_window_title(&format!("devdeck · {}", shell.read(cx).active_name()));
            if !app_settings::is_open(cx) {
                match self.remembered.get(&self.deck.entity_id()).and_then(WeakFocusHandle::upgrade) {
                    Some(h) => h.focus(window, cx),
                    None => self.focus_panes(window, cx),
                }
            }
        }
        cx.notify();
    }

    /// Opens or closes the full-window Settings view (main window only), moving focus
    /// into it, and back to where it was when it closes.
    fn set_settings_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_main() || open == app_settings::is_open(cx) {
            return;
        }
        app_settings::set_open(open, cx);
        if open {
            self.before_settings = window.focused(cx).map(|f| f.downgrade());
            let view = self.settings.get_or_insert_with(|| cx.new(SettingsView::new)).clone();
            view.read(cx).focus_handle(cx).focus(window, cx);
        } else {
            match self.before_settings.take().and_then(|h| h.upgrade()) {
                Some(h) => h.focus(window, cx),
                None => self.focus_panes(window, cx),
            }
        }
        cx.notify();
    }

    /// Starts "Send to…" for the focused pane's active tab, or the whole pane.
    fn start_send(&mut self, pane: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.focused_pane(cx) else { return };
        let source = if pane {
            SendSource::Pane(p)
        } else {
            match self.deck.read(cx).layout().pane(p).ok().and_then(|p| p.active_view().cloned()) {
                Some(v) => SendSource::Tab(v),
                None => return,
            }
        };
        crate::send_to::open(self.deck.clone(), source, window, cx);
    }

    /// shift+F10 / Menu key: the menu of what has focus, placed next to it.
    fn open_context_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let t = theme(cx).clone();
        if let Some(sidebar) = self.active_sidebar()
            && Self::sidebar_has_focus(&sidebar, window, cx)
        {
            let s = sidebar.read(cx);
            if let Some(key) = s.selected.clone() {
                let pinned = s.state.pins.values().any(|p| p.contains(&key));
                let items = Sidebar::item_menu(&sidebar, &key, pinned, cx);
                open_menu(point(t.activity_w + t.size(40.), t.title_h + t.size(90.)), items, window, cx);
            }
            return;
        }
        let Some(p) = self.focused_pane(cx) else { return };
        let (rect, active) = {
            let d = self.deck.read(cx);
            let size = self.body_size(window, &t, cx);
            let rect = d.layout().pane_rects(self.id, size, f32::from(t.tab_h)).ok().and_then(|r| r.into_iter().find(|(q, _)| *q == p)).map(|(_, r)| r);
            (rect, d.layout().pane(p).map(|p| p.active).unwrap_or(0))
        };
        let origin = self.panes_origin.get();
        let at = rect.map_or(origin, |r| point(origin.x + logical(r.x) + t.size(24.), origin.y + logical(r.y) + t.tab_h));
        let d = self.deck.read(cx);
        let items = tab_menu(&self.deck, p, active, self.id, d.layout(), &d.book.keep);
        open_menu(at, items, window, cx);
    }

    /// ctrl+k: the palette opens in the main window (brought forward from a detached one).
    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_main() {
            if !app_settings::is_open(cx) {
                crate::palette::open(window, cx);
            }
            return;
        }
        if let Some(main) = shell::try_shell(cx).and_then(|s| s.read(cx).main_window()) {
            cx.defer(move |cx| {
                let _ = main.update(cx, |_, window, cx| {
                    window.activate_window();
                    crate::palette::open(window, cx);
                });
            });
        }
    }

    fn active_sidebar(&self) -> Option<Entity<Sidebar>> {
        self.sidebar.as_ref().map(|(s, _)| s.clone())
    }

    /// Shows an activity and focuses its list (ctrl+1/2/3, activity bar).
    fn show_activity(&mut self, activity: Activity, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sidebar) = self.active_sidebar() else { return };
        if app_settings::is_open(cx) {
            return;
        }
        if sidebar.update(cx, |s, cx| s.show(activity, cx)) {
            sidebar.read(cx).list_focus.clone().focus(window, cx);
        }
    }

    /// Shows or hides the sidebar; hiding it while it has focus sends focus to the panes.
    fn set_sidebar_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sidebar) = self.active_sidebar() else { return };
        if app_settings::is_open(cx) {
            return;
        }
        let had_focus = Self::sidebar_has_focus(&sidebar, window, cx);
        sidebar.update(cx, |s, cx| s.set_visible(visible, cx));
        if visible {
            sidebar.read(cx).list_focus.clone().focus(window, cx);
        } else if had_focus {
            self.focus_panes(window, cx);
        }
    }

    fn sidebar_has_focus(sidebar: &Entity<Sidebar>, window: &Window, cx: &App) -> bool {
        let s = sidebar.read(cx);
        s.list_focus.is_focused(window) || s.filter.read(cx).focus_handle(cx).is_focused(window)
    }

    fn open_env_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sidebar) = self.active_sidebar() else { return };
        if sidebar.read(cx).envs(cx).is_empty() || app_settings::is_open(cx) {
            return;
        }
        let t = theme(cx);
        let at = point(t.size(190.), t.title_h);
        open_menu(at, Sidebar::env_menu(&sidebar, cx), window, cx);
    }

    fn apply(&self, cx: &mut App, op: impl FnOnce(&mut Layout) -> devdeck_core::layout::Result<()>) {
        self.deck.update(cx, |d, cx| {
            d.apply(cx, op);
        });
    }

    fn focused_pane(&self, cx: &App) -> Option<PaneId> {
        self.deck.read(cx).layout().window(self.id).ok().map(|w| w.focused)
    }

    /// The pane area's size: the window body minus the activity bar and sidebar.
    fn body_size(&self, window: &Window, t: &Theme, cx: &App) -> (f32, f32) {
        let s = window.viewport_size();
        let mut w = s.width;
        if let Some(sidebar) = self.active_sidebar() {
            w -= t.activity_w;
            if sidebar.read(cx).state.visible {
                w -= t.sidebar_w;
            }
        }
        (f32::from(w), f32::from(s.height - t.title_h))
    }

    fn focus_dir(&mut self, dir: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.focused_pane(cx) else { return };
        let t = theme(cx);
        let (size, strip) = (self.body_size(window, t, cx), f32::from(t.tab_h));
        self.apply(cx, |l| {
            if let Some(n) = l.neighbor(p, dir, size, strip)? {
                l.focus(n)?;
            }
            Ok(())
        });
    }

    fn move_tab_dir(&mut self, dir: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.focused_pane(cx) else { return };
        let t = theme(cx);
        let (size, strip) = (self.body_size(window, t, cx), f32::from(t.tab_h));
        self.apply(cx, |l| {
            if l.pane(p)?.tabs.is_empty() {
                return Ok(());
            }
            let idx = l.pane(p)?.active;
            match l.neighbor(p, dir, size, strip)? {
                Some(n) => l.move_tab(p, idx, Drop::Into(n)),
                None => {
                    let edge = match dir {
                        Direction::Left => Edge::Left,
                        Direction::Right => Edge::Right,
                        Direction::Up => Edge::Top,
                        Direction::Down => Edge::Bottom,
                    };
                    l.move_tab(p, idx, Drop::Edge(p, edge))
                }
            }
        });
    }

    fn with_focused(&mut self, cx: &mut Context<Self>, op: impl FnOnce(&mut Layout, PaneId) -> devdeck_core::layout::Result<()>) {
        if let Some(p) = self.focused_pane(cx) {
            self.apply(cx, |l| op(l, p));
        }
    }

    /// A new terminal in the focused pane, in its active tab's app folder (or the
    /// workspace's first folder), with the keys.
    fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.focused_pane(cx) else { return };
        self.deck.update(cx, |d, cx| {
            let dir = d.terminal_folder(Some(pane), cx);
            d.open_terminal(&dir, Some(pane), cx);
        });
        if let Some(f) = self.deck.read(cx).view_focus(pane) {
            f.focus(window, cx);
        }
    }

    fn reattach(&mut self, cx: &mut Context<Self>) {
        let id = self.id;
        self.deck.update(cx, |d, cx| d.reattach(id, cx));
    }
}

fn split_active(l: &mut Layout, p: PaneId, edge: Edge) -> devdeck_core::layout::Result<()> {
    let pane = l.pane(p)?;
    if pane.tabs.len() < 2 {
        return Ok(());
    }
    let i = pane.active;
    l.move_tab(p, i, Drop::Edge(p, edge))
}

impl Render for WindowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        stats::count_render();
        let t = theme(cx).clone();
        apply_window_scale(window, &t);
        // Terminals answer colour and size queries with the theme even before a run view
        // painted one (a run started in the background).
        devdeck_ui::terminal::sync_look(&t, devdeck_ui::terminal::cell_size(window, &t));
        let is_main = self.is_main();
        let settings_open = is_main && app_settings::is_open(cx);
        if settings_open && self.settings.is_none() {
            self.settings = Some(cx.new(SettingsView::new));
        }
        let (layout, views) = {
            let d = self.deck.read(cx);
            let views: HashMap<PaneId, AnyView> = d
                .layout()
                .panes
                .iter()
                .filter_map(|(id, p)| p.active_view().and_then(|r| d.view(r)).map(|v| (*id, v)))
                .collect();
            (d.layout().clone(), views)
        };
        let Ok(w) = layout.window(self.id) else {
            return div().size_full().bg(t.bg).into_any_element();
        };
        let numbers: HashMap<PaneId, (usize, bool)> = {
            let d = self.deck.read(cx);
            if d.send.is_some() {
                layout.numbered_panes().into_iter().enumerate().map(|(i, (p, _))| (p, (i + 1, d.send_refusal(Some(p)).is_none()))).collect()
            } else {
                HashMap::new()
            }
        };
        let view_focus: HashMap<PaneId, FocusHandle> = {
            let d = self.deck.read(cx);
            layout.panes.keys().map(|p| (*p, d.view_focus(*p).unwrap_or_else(|| self.panes_focus.clone()))).collect()
        };
        let frame = Frame { layout: &layout, numbers: &numbers, view_focus: &view_focus, views: &views, window: self.id, deck: &self.deck, t: &t, dragging: cx.has_active_drag(), tab_insert: &self.tab_insert };
        let deck_id = self.deck.entity_id();
        let workspace = self.deck.read(cx).workspace.clone();
        let shell = self.shell.as_ref().and_then(WeakEntity::upgrade);
        let ws_name = shell.as_ref().map_or(workspace, |s| s.read(cx).active_name().to_string());
        let tree = match w.maximized {
            Some(p) => render_pane(&frame, p, None),
            None => render_node(&frame, &w.root, None),
        };
        let id = self.id;
        let close_deck = self.deck.clone();
        let menu_shell = shell.clone();
        let sidebar = self.active_sidebar();
        let env = sidebar.as_ref().and_then(|s| s.read(cx).env(cx));

        let title_bar = div()
            .id("titlebar")
            .flex_none()
            .h(t.title_h)
            .px_3()
            .flex()
            .items_center()
            .gap_3()
            .bg(t.bar)
            .border_b_1()
            .border_color(t.line)
            .on_mouse_down(MouseButton::Left, |e, window, _| {
                if e.click_count == 2 {
                    window.zoom_window();
                } else {
                    window.start_window_move();
                }
            })
            .child(div().text_color(t.hi).child("devdeck"))
            .when_some(menu_shell, |d, shell| {
                d.child(
                    button("workspace", format!("{ws_name} ▾"), &t)
                        .debug_selector(|| "workspace-button".into())
                        .tooltip(tip("switch workspace"))
                        .on_click({
                            let shell = shell.clone();
                            move |e, window, cx| {
                                let items = workspace_menu_items(&shell, cx);
                                open_menu(e.position(), items, window, cx);
                            }
                        })
                        .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                            let items = workspace_menu_items(&shell, cx);
                            open_menu(e.position, items, window, cx);
                        }),
                )
            })
            .when_some(env, |d, env| {
                let sidebar = sidebar.clone();
                let text = if env.protected { format!("⊘ @{} ▾", env.name) } else { format!("@{} ▾", env.name) };
                d.child(
                    button("env", text, &t)
                        .text_color(if env.protected { t.err } else { t.env })
                        .debug_selector(|| "env-button".into())
                        .tooltip(tip("env for runs and the cluster list"))
                        .on_click({
                            let sidebar = sidebar.clone();
                            move |e, window, cx| {
                                if let Some(s) = &sidebar {
                                    open_menu(e.position(), Sidebar::env_menu(s, cx), window, cx);
                                }
                            }
                        })
                        .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                            if let Some(s) = &sidebar {
                                open_menu(e.position, Sidebar::env_menu(s, cx), window, cx);
                            }
                        }),
                )
            })
            .when(is_main, |d| {
                let deck = self.deck.clone();
                let current = deck.read(cx).book.current.clone();
                d.child(
                    button("layout", format!("{current} ▾"), &t)
                        .debug_selector(|| "layout-button".into())
                        .tooltip(tip("layouts"))
                        .on_click(move |e, window, cx| {
                            let items = crate::layouts::menu(&deck, cx);
                            open_menu(e.position(), items, window, cx);
                        }),
                )
            })
            .when(!is_main, |d| d.child(div().text_color(t.mute).child(format!("{ws_name} · detached window"))))
            .child(div().flex_1())
            .when(is_main, |d| {
                d.child(
                    button("search", "search", &t)
                        .debug_selector(|| "search-button".into())
                        .on_click(|_, window, cx| crate::palette::open(window, cx)),
                )
                .child(
                    button("keys", "?", &t)
                        .tooltip(tip("keyboard shortcuts"))
                        .on_click(|_, window, cx| crate::cheat_sheet::open(window, cx)),
                )
            })
            .when(is_main, |d| {
                d.child(
                    button("settings", if settings_open { "⚙ settings ✓" } else { "⚙ settings" }, &t)
                        .tooltip(tip("settings"))
                        .on_click(cx.listener(move |this, _, window, cx| this.set_settings_open(!settings_open, window, cx))),
                )
            })
            .when(!is_main, |d| {
                d.child(
                    button("reattach", "⇲", &t)
                        .tooltip(tip("send back to the main window"))
                        .on_click(cx.listener(|this, _, _, cx| this.reattach(cx))),
                )
            })
            .child(button("min", "—", &t).on_click(|_, window, _| window.minimize_window()))
            .child(button("max", "□", &t).on_click(|_, window, _| window.zoom_window()))
            .child(button("close", "×", &t).on_click(move |_, _, cx| {
                if id == MAIN_WINDOW {
                    shell::request_quit(cx);
                } else {
                    close_deck.update(cx, |d, cx| d.close_window(id, cx));
                }
            }));

        let panes = div()
            .id("panes")
            .relative()
            .key_context("Panes")
            .track_focus(&self.panes_focus)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .on_action(cx.listener(|this, _: &FocusLeft, w, cx| this.focus_dir(Direction::Left, w, cx)))
            .on_action(cx.listener(|this, _: &FocusRight, w, cx| this.focus_dir(Direction::Right, w, cx)))
            .on_action(cx.listener(|this, _: &FocusUp, w, cx| this.focus_dir(Direction::Up, w, cx)))
            .on_action(cx.listener(|this, _: &FocusDown, w, cx| this.focus_dir(Direction::Down, w, cx)))
            .on_action(cx.listener(|this, _: &MoveTabLeft, w, cx| this.move_tab_dir(Direction::Left, w, cx)))
            .on_action(cx.listener(|this, _: &MoveTabRight, w, cx| this.move_tab_dir(Direction::Right, w, cx)))
            .on_action(cx.listener(|this, _: &MoveTabUp, w, cx| this.move_tab_dir(Direction::Up, w, cx)))
            .on_action(cx.listener(|this, _: &MoveTabDown, w, cx| this.move_tab_dir(Direction::Down, w, cx)))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.with_focused(cx, |l, p| l.cycle_tab(p, 1))))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| this.with_focused(cx, |l, p| l.cycle_tab(p, -1))))
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| {
                this.with_focused(cx, |l, p| {
                    if l.pane(p)?.tabs.is_empty() {
                        return Ok(());
                    }
                    let i = l.pane(p)?.active;
                    l.close_tab(p, i).map(|_| ())
                })
            }))
            .on_action(cx.listener(|this, _: &NewTerminal, window, cx| this.new_terminal(window, cx)))
            .on_action(cx.listener(|this, _: &SplitRight, _, cx| this.with_focused(cx, |l, p| split_active(l, p, Edge::Right))))
            .on_action(cx.listener(|this, _: &SplitDown, _, cx| this.with_focused(cx, |l, p| split_active(l, p, Edge::Bottom))))
            .on_action(cx.listener(|this, _: &ToggleCollapse, _, cx| this.with_focused(cx, |l, p| l.toggle_collapse(p))))
            .on_action(cx.listener(|this, _: &ToggleMaximize, _, cx| this.with_focused(cx, |l, p| l.toggle_maximize(p))))
            .on_action(cx.listener(|this, _: &DetachTab, _, cx| {
                this.with_focused(cx, |l, p| {
                    if l.pane(p)?.tabs.is_empty() {
                        return Ok(());
                    }
                    let i = l.pane(p)?.active;
                    l.detach(p, i).map(|_| ())
                })
            }))
            .on_action(cx.listener(|this, _: &SendTab, w, cx| this.start_send(false, w, cx)))
            .on_action(cx.listener(|this, _: &SendPane, w, cx| this.start_send(true, w, cx)))
            .on_action(cx.listener(|this, _: &GrowPane, _, cx| this.with_focused(cx, |l, p| l.grow(p, 0.05))))
            .on_action(cx.listener(|this, _: &ShrinkPane, _, cx| this.with_focused(cx, |l, p| l.grow(p, -0.05))))
            // Per-workspace id, so element state never carries over between workspaces.
            .child(div().id(("ws", deck_id.as_u64())).size_full().min_w_0().min_h_0().flex().child(tree))
            .child({
                let origin = self.panes_origin.clone();
                canvas(move |bounds, _, _| origin.set(bounds.origin), |_, _, _, _| {}).absolute().size_full()
            });
        let body = match (&self.settings, settings_open) {
            (Some(view), true) => div().flex_1().min_h_0().min_w_0().flex().child(view.clone()).into_any_element(),
            _ => match self.active_sidebar() {
                Some(sidebar) => {
                    let visible = sidebar.read(cx).state.visible;
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .flex()
                        .child(activity_bar(&sidebar, &t, cx))
                        .when(visible, |d| {
                            // Redrawn only when the sidebar itself changes, not on every
                            // frame of a streaming view next to it.
                            d.child(sidebar.clone().cached(StyleRefinement::default().flex_none().w(t.sidebar_w).h_full()))
                        })
                        .child(panes)
                        .into_any_element()
                }
                None => panes.into_any_element(),
            },
        };

        let root = div()
            .id("window-root")
            .track_focus(&self.focus)
            .key_context("Deck")
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.fg)
            .font_family(t.font.clone())
            .text_size(t.text)
            .line_height(t.line_height)
            .on_action(|_: &IncreaseScale, _, cx| app_settings::increase(cx))
            .on_action(|_: &DecreaseScale, _, cx| app_settings::decrease(cx))
            .on_action(|_: &ResetScale, _, cx| app_settings::reset(cx))
            .on_action(|_: &CheatSheet, window, cx| crate::cheat_sheet::open(window, cx))
            .on_action(cx.listener(|this, _: &Palette, w, cx| this.open_palette(w, cx)))
            // Only where they do something (R5.1: unavailable actions aren't offered).
            .when(is_main, |d| {
                d.on_action(cx.listener(|this, _: &ToggleSettings, w, cx| {
                    let open = app_settings::is_open(cx);
                    this.set_settings_open(!open, w, cx);
                }))
                .on_action(cx.listener(|this, _: &CloseSettings, w, cx| this.set_settings_open(false, w, cx)))
            })
            .when(!is_main, |d| d.on_action(cx.listener(|this, _: &Reattach, _, cx| this.reattach(cx))))
            .when(is_main && !settings_open, |d| {
                d.on_action(cx.listener(|this, _: &ShowApps, w, cx| this.show_activity(Activity::Apps, w, cx)))
                    .on_action(cx.listener(|this, _: &ShowCluster, w, cx| this.show_activity(Activity::Cluster, w, cx)))
                    .on_action(cx.listener(|this, _: &ShowPorts, w, cx| this.show_activity(Activity::Ports, w, cx)))
                    .on_action(cx.listener(|this, _: &ShowLayouts, w, cx| this.show_activity(Activity::Layouts, w, cx)))
                    .on_action(cx.listener(|this, _: &ShowTerminals, w, cx| this.show_activity(Activity::Terminals, w, cx)))
                    .on_action(cx.listener(|this, _: &SaveLayoutAs, w, cx| crate::layouts::open_save_as(this.deck.clone(), w, cx)))
                    .on_action(cx.listener(|this, _: &ToggleSidebar, w, cx| {
                        let visible = this.active_sidebar().is_some_and(|s| s.read(cx).state.visible);
                        this.set_sidebar_visible(!visible, w, cx);
                    }))
                    .on_action(cx.listener(|this, _: &OpenEnvMenu, w, cx| this.open_env_menu(w, cx)))
                    // Esc with an empty filter: back to the panes.
                    .on_action(cx.listener(|this, _: &SidebarEscape, w, cx| this.focus_panes(w, cx)))
            })
            .when(!settings_open, |d| d.on_action(cx.listener(|this, _: &ContextMenu, w, cx| this.open_context_menu(w, cx))))
            .child(title_bar)
            .child(body)
            .children(devdeck_ui::render_layer(window, cx))
            .when(is_main, |d| d.child(devdeck_ui::toasts(cx)));

        match window.window_decorations() {
            Decorations::Server => root.into_any_element(),
            decorations @ Decorations::Client { .. } => {
                let edge = t.window_edge;
                let frame = frame_color(decorations, window.is_maximized(), window.is_fullscreen(), window.is_window_active(), &t);
                div()
                .id("csd")
                .size_full()
                .relative()
                .when_some(frame, |d, color| d.border_1().border_color(color))
                .capture_any_mouse_down(move |e, window, cx| {
                    // Left button only, like the compositor's own borders.
                    if e.button != MouseButton::Left {
                        return;
                    }
                    if let Some(edge) = window_edge_at(window, e.position, edge) {
                        window.start_window_resize(edge);
                        cx.stop_propagation();
                    }
                })
                .child(root)
                .child(window_edge_cursors(edge))
                .into_any_element()
            }
        }
    }
}

/// The activity bar: one button per activity the workspace offers. Clicking the shown
/// activity hides the sidebar; clicking another shows it.
fn activity_bar(sidebar: &Entity<Sidebar>, t: &Theme, cx: &mut Context<WindowView>) -> impl IntoElement {
    let s = sidebar.read(cx);
    let (current, visible) = (s.activity(), s.state.visible);
    let mut bar = div().flex_none().w(t.activity_w).h_full().flex().flex_col().items_center().pt_1().gap_1().bg(t.bar).border_r_1().border_color(t.line);
    for a in s.activities(cx) {
        let on = visible && a == current;
        let hover = t.hover;
        bar = bar.child(
            div()
                .id(("activity", a as u64))
                .debug_selector(move || format!("activity-{}", a.id()))
                .w(t.activity_w)
                .h(t.activity_w)
                .flex()
                .items_center()
                .justify_center()
                .text_size(t.size(18.))
                .text_color(if on { t.hi } else { t.mute })
                .when(on, |d| d.border_l_2().border_color(t.accent))
                .hover(move |s| s.bg(hover))
                .cursor_pointer()
                .tooltip(tip(a.title()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if on {
                        this.set_sidebar_visible(false, window, cx);
                    } else {
                        this.show_activity(a, window, cx);
                    }
                }))
                .on_mouse_down(MouseButton::Right, {
                    let me = cx.entity();
                    move |e, window, cx| {
                        let (m1, m2) = (me.clone(), me.clone());
                        let items = vec![
                            MenuItem::new(format!("Show {}", a.title()), move |window, cx| m1.update(cx, |v, cx| v.show_activity(a, window, cx))),
                            MenuItem::new("Hide sidebar", move |window, cx| m2.update(cx, |v, cx| v.set_sidebar_visible(false, window, cx))),
                        ];
                        open_menu(e.position, items, window, cx);
                    }
                })
                .child(a.glyph()),
        );
    }
    bar
}

/// Resize cursors along the window border (client-side decorations; GNOME on Wayland
/// draws none): one hitbox per band, so GPUI picks the cursor as the pointer moves,
/// without a redraw. The resize itself starts in `capture_any_mouse_down`.
fn window_edge_cursors(edge_width: Pixels) -> impl IntoElement {
    canvas(
        move |_bounds, window, _cx| {
            // Corners last: they sit on top of the side bands they overlap.
            let mut bands = edge_bands(window.viewport_size(), tiling(window), edge_width);
            bands.reverse();
            bands.into_iter().map(|(edge, b)| (edge, window.insert_hitbox(b, HitboxBehavior::Normal))).collect::<Vec<_>>()
        },
        |_bounds, hitboxes, window, _cx| {
            for (edge, hitbox) in &hitboxes {
                let style = match edge {
                    ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                    ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
                    ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
                    ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
                };
                window.set_cursor_style(style, hitbox);
            }
        },
    )
    .absolute()
    .size_full()
}

/// The window edge under `pos`, if the window can be resized from there.
fn window_edge_at(window: &Window, pos: Point<Pixels>, edge: Pixels) -> Option<ResizeEdge> {
    resize_edge(pos, window.viewport_size(), tiling(window), edge)
}

fn resize_edge(pos: Point<Pixels>, size: Size<Pixels>, tiling: Tiling, edge: Pixels) -> Option<ResizeEdge> {
    edge_bands(size, tiling, edge).into_iter().find(|(_, b)| b.contains(&pos)).map(|(e, _)| e)
}

/// Where the window can be resized from, corners first: a band `edge` wide along each
/// side the compositor doesn't hold (see [`tiling`]), a corner where two such sides meet.
/// `size` is the drawn size (`viewport_size`): `window_bounds()` of a maximized window
/// is its restore size on Wayland, which once made most of the window an "edge".
fn edge_bands(size: Size<Pixels>, tiling: Tiling, edge: Pixels) -> Vec<(ResizeEdge, Bounds<Pixels>)> {
    let (w, h) = (size.width, size.height);
    let b = |x, y, bw, bh| Bounds::new(point(x, y), gpui::size(bw, bh));
    let (top, bottom, left, right) = (!tiling.top, !tiling.bottom, !tiling.left, !tiling.right);
    let corners = [
        (top && left, ResizeEdge::TopLeft, b(Pixels::ZERO, Pixels::ZERO, edge, edge)),
        (top && right, ResizeEdge::TopRight, b(w - edge, Pixels::ZERO, edge, edge)),
        (bottom && left, ResizeEdge::BottomLeft, b(Pixels::ZERO, h - edge, edge, edge)),
        (bottom && right, ResizeEdge::BottomRight, b(w - edge, h - edge, edge, edge)),
    ];
    let sides = [
        (top, ResizeEdge::Top, b(Pixels::ZERO, Pixels::ZERO, w, edge)),
        (bottom, ResizeEdge::Bottom, b(Pixels::ZERO, h - edge, w, edge)),
        (left, ResizeEdge::Left, b(Pixels::ZERO, Pixels::ZERO, edge, h)),
        (right, ResizeEdge::Right, b(w - edge, Pixels::ZERO, edge, h)),
    ];
    corners.into_iter().chain(sides).filter(|(on, ..)| *on).map(|(_, e, r)| (e, r)).collect()
}

fn eid(name: &'static str, a: u64, b: usize) -> (&'static str, u64) {
    (name, a * 10_000 + b as u64)
}

// ---------------------------------------------------------------------- pane tree

fn render_node(f: &Frame, node: &Node, parent: Option<Axis>) -> AnyElement {
    let s = match node {
        Node::Pane(p) => return render_pane(f, *p, parent),
        Node::Split(s) => s,
    };
    let (split_id, axis) = (s.id, s.axis);
    let deck = f.deck.clone();
    let deck_id = deck.entity_id();
    let mut container = div()
        .id(("split", split_id.0))
        .size_full()
        .min_w_0()
        .min_h_0()
        .flex()
        .when(axis == Axis::Column, |d| d.flex_col())
        .on_drag_move::<DraggedDivider>(move |e: &DragMoveEvent<DraggedDivider>, _, cx| {
            let d = e.drag(cx).clone();
            if d.split != split_id || d.deck != deck_id {
                return;
            }
            let (pos, origin, len) = match d.axis {
                Axis::Row => (e.event.position.x, e.bounds.origin.x, e.bounds.size.width),
                Axis::Column => (e.event.position.y, e.bounds.origin.y, e.bounds.size.height),
            };
            if len <= Pixels::ZERO {
                return;
            }
            let fraction = f32::from(pos - origin) / f32::from(len);
            deck.update(cx, |deck, cx| {
                deck.apply(cx, |l| l.set_boundary(split_id, d.index, fraction));
            });
        });
    let n = s.children.len();
    let is_collapsed = |node: &Node| matches!(node, Node::Pane(p) if f.layout.pane(*p).is_ok_and(|p| p.collapsed));
    // Grow factors below 1 leave free space unused, so normalize over expanded children.
    let expanded: f32 = s.children.iter().filter(|c| !is_collapsed(&c.node)).map(|c| c.weight).sum();
    for (i, c) in s.children.iter().enumerate() {
        let collapsed = is_collapsed(&c.node);
        let slot = div().min_w_0().min_h_0().overflow_hidden().flex();
        let slot = if collapsed {
            match axis {
                Axis::Row => slot.flex_none().w(f.t.rail_w),
                Axis::Column => slot.flex_none().h(f.t.tab_h),
            }
        } else {
            slot.flex_basis(Pixels::ZERO).flex_grow(c.weight / expanded.max(f32::EPSILON))
        };
        container = container.child(slot.child(render_node(f, &c.node, Some(axis))));
        if i + 1 < n {
            container = container.child(divider(deck_id, split_id, i, axis, f.t));
        }
    }
    container.into_any_element()
}

/// 5 px grab area with a 1 px line in the middle.
fn divider(deck: EntityId, split: SplitId, index: usize, axis: Axis, t: &Theme) -> impl IntoElement {
    let (line, grab, hair) = (t.line, t.divider_grab, t.hairline);
    let base = div()
        .id(eid("divider", split.0, index))
        .flex_none()
        .flex()
        .justify_center()
        .items_center()
        .on_drag(DraggedDivider { deck, split, index, axis }, |_, _, _, cx| cx.new(|_| Empty));
    match axis {
        Axis::Row => base.w(grab).h_full().cursor_col_resize().child(div().w(hair).h_full().bg(line)),
        Axis::Column => base.h(grab).w_full().cursor_row_resize().child(div().h(hair).w_full().bg(line)),
    }
}

fn apply_on(deck: &Entity<Deck>, cx: &mut App, op: impl FnOnce(&mut Layout) -> devdeck_core::layout::Result<()>) {
    deck.update(cx, |d, cx| {
        d.apply(cx, op);
    });
}

fn render_pane(f: &Frame, pane: PaneId, parent: Option<Axis>) -> AnyElement {
    let inner = render_pane_inner(f, pane, parent);
    let Some(&(n, enabled)) = f.numbers.get(&pane) else { return inner };
    // Send to: this pane's number; clicking it completes the pick.
    let t = f.t;
    let deck = f.deck.clone();
    // The whole pane is the target while a pick runs (R3.11: click the pane).
    let target = div()
        .id(("send-target", pane.0))
        .debug_selector(move || format!("send-target-{n}"))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(t.drop_bg)))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            if enabled {
                deck.update(cx, |d, cx| d.send_to(Some(pane), cx));
            }
        });
    let badge = div()
        .debug_selector(move || format!("send-badge-{n}"))
        .absolute()
        .top(t.size(4.))
        .left(t.size(4.))
        .min_w(t.size(28.))
        .h(t.size(28.))
        .px_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded(t.radius_m)
        .bg(if enabled { t.accent } else { t.line })
        .text_color(if enabled { t.bg } else { t.mute })
        .text_size(t.size(16.))
        .child(n.to_string());
    div().relative().size_full().min_w_0().min_h_0().flex().child(inner).child(target.child(badge)).into_any_element()
}

/// The menu of a tab (right-click, shift+F10).
fn tab_menu(deck: &Entity<Deck>, pane: PaneId, idx: usize, window: WindowId, layout: &Layout, keep: &std::collections::BTreeSet<ViewRef>) -> Vec<MenuItem> {
    let Some(view) = layout.pane(pane).ok().and_then(|p| p.tabs.get(idx).cloned()) else { return Vec::new() };
    let alone = layout.is_detached(window) && layout.pane(pane).is_ok_and(|p| p.tabs.len() == 1);
    let maximized = layout.window(window).is_ok_and(|w| w.maximized == Some(pane));
    let d = |deck: &Entity<Deck>| deck.clone();
    let (d1, d2, d3, d4, d5, d6) = (d(deck), d(deck), d(deck), d(deck), d(deck), d(deck));
    let (v1, v2, v3) = (view.clone(), view.clone(), view.clone());
    let kept = keep.contains(&view);
    let d7 = deck.clone();
    let mut detach = MenuItem::new("Detach to a window", move |_, cx| apply_on(&d3, cx, |l| l.detach(pane, idx).map(|_| ())));
    if alone {
        detach = detach.disabled("already alone in its window");
    }
    vec![
        MenuItem::new("Close", move |_, cx| apply_on(&d1, cx, |l| l.close_view(&v1))),
        MenuItem::new("Close others", move |_, cx| {
            apply_on(&d2, cx, |l| {
                let others: Vec<_> = l.pane(pane)?.tabs.iter().filter(|r| **r != v2).cloned().collect();
                for r in others {
                    l.close_view(&r)?;
                }
                Ok(())
            })
        }),
        MenuItem::separator(),
        MenuItem::new("Send to…", move |window, cx| crate::send_to::open(d4.clone(), SendSource::Tab(view.clone()), window, cx)),
        detach,
        MenuItem::separator(),
        MenuItem::new("Maximize", move |_, cx| apply_on(&d5, cx, |l| l.toggle_maximize(pane))).checked(maximized),
        MenuItem::new("Collapse", move |_, cx| apply_on(&d6, cx, |l| l.toggle_collapse(pane))),
        MenuItem::separator(),
        MenuItem::new("Keep in every layout", move |_, cx| {
            d7.update(cx, |d, cx| {
                d.toggle_keep(&v3, cx);
            })
        })
        .checked(kept),
    ]
}

/// The menu of a pane (right-click on its tab strip or grip).
fn pane_menu(deck: &Entity<Deck>, pane: PaneId, layout: &Layout) -> Vec<MenuItem> {
    let tabs = layout.pane(pane).map(|p| p.tabs.len()).unwrap_or(0);
    let d = |deck: &Entity<Deck>| deck.clone();
    let (d1, d2, d3, d4, d5, d6) = (d(deck), d(deck), d(deck), d(deck), d(deck), d(deck));
    let split = |item: MenuItem| if tabs < 2 { item.disabled("needs two tabs") } else { item };
    vec![
        split(MenuItem::new("Split right", move |_, cx| apply_on(&d1, cx, |l| split_active(l, pane, Edge::Right)))),
        split(MenuItem::new("Split down", move |_, cx| apply_on(&d2, cx, |l| split_active(l, pane, Edge::Bottom)))),
        MenuItem::separator(),
        MenuItem::new("Collapse", move |_, cx| apply_on(&d3, cx, |l| l.toggle_collapse(pane))),
        MenuItem::new("Maximize", move |_, cx| apply_on(&d4, cx, |l| l.toggle_maximize(pane))),
        MenuItem::new("Send pane to…", move |window, cx| crate::send_to::open(d5.clone(), SendSource::Pane(pane), window, cx)),
        MenuItem::separator(),
        MenuItem::new("Close pane", move |_, cx| {
            apply_on(&d6, cx, |l| {
                let views = l.pane(pane)?.tabs.clone();
                for v in views {
                    l.close_view(&v)?;
                }
                Ok(())
            })
        }),
    ]
}

fn render_pane_inner(f: &Frame, pane: PaneId, parent: Option<Axis>) -> AnyElement {
    let Ok(p) = f.layout.pane(pane) else { return div().into_any_element() };
    let t = f.t;
    let win = f.layout.window(f.window).ok();
    let focused = win.is_some_and(|w| w.focused == pane);
    let maximized = win.is_some_and(|w| w.maximized == Some(pane));
    let titles: Vec<SharedString> = p.tabs.iter().map(|r| views::title(r).into()).collect();
    let deck_id = f.deck.entity_id();

    // Collapsed in a row split: a narrow rail of monograms (GPUI can't rotate text).
    if p.collapsed && parent == Some(Axis::Row) {
        let mut rail = div().id(("rail", pane.0)).size_full().flex().flex_col().items_center().gap_1().pt_1().bg(t.bar);
        for (i, title) in titles.iter().enumerate() {
            let deck = f.deck.clone();
            let full = title.clone();
            let mono: String = title.chars().filter(|c| c.is_alphanumeric()).take(2).collect();
            let hover = t.hover;
            rail = rail.child(
                div()
                    .id(eid("rail-tab", pane.0, i))
                    .w(t.icon_button)
                    .h(t.icon_button)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(t.radius_s)
                    .text_color(if i == p.active { t.fg } else { t.mute })
                    .hover(move |s| s.bg(hover))
                    .cursor_pointer()
                    .tooltip(tip(full))
                    .on_click(move |_, _, cx| apply_on(&deck, cx, |l| l.activate(pane, i)))
                    .child(mono),
            );
        }
        return rail.into_any_element();
    }

    // Tab strip: drop target for tabs (move into this pane).
    let drop_bg = t.drop_bg;
    let strip_deck = f.deck.clone();
    let mut strip = div()
        .id(("strip", pane.0))
        .flex_none()
        .h(t.tab_h)
        .w_full()
        .min_w_0()
        .flex()
        .bg(t.bar)
        .border_b_1()
        .border_color(t.line)
        .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(drop_bg))
        .on_mouse_down(MouseButton::Right, {
            let deck = f.deck.clone();
            move |e, window, cx| {
                let layout = deck.read(cx).layout().clone();
                open_menu(e.position, pane_menu(&deck, pane, &layout), window, cx);
            }
        })
        .on_drop({
            let insert = f.tab_insert.clone();
            move |tab: &DraggedTab, _, cx| {
                insert.set(None);
                if tab.deck == deck_id {
                    let tab = tab.clone();
                    apply_on(&strip_deck, cx, |l| l.move_tab(tab.pane, tab.index, Drop::Into(pane)));
                }
            }
        });

    // Grip: drag the whole pane.
    let pane_title: SharedString = titles.get(p.active).cloned().unwrap_or_else(|| "empty".into());
    strip = strip.child(
        div()
            .id(("grip", pane.0))
            .flex_none()
            .w(t.grip_w)
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(t.dim)
            .cursor_grab()
            .tooltip(tip("drag to move this pane"))
            .on_drag(DraggedPane { deck: deck_id, pane, title: pane_title }, |d, _, _, cx| {
                let label: SharedString = format!("pane: {}", d.title).into();
                cx.new(|_| DragGhost(label))
            })
            .child("⠿"),
    );

    // Tabs keep priority over the pane buttons when the pane is narrow.
    let mut tabs = div().flex().flex_1().min_w(t.tabs_min_w).overflow_hidden();
    for (i, title) in titles.iter().enumerate() {
        let active = i == p.active;
        let (d_click, d_mid, d_close) = (f.deck.clone(), f.deck.clone(), f.deck.clone());
        let full = title.clone();
        let (hover, line, fg, accent) = (t.hover, t.line, t.fg, t.accent);
        // Dragged over, a tab is a place: its left half before it, its right half after.
        let (moving, showing, dropping) = (f.tab_insert.clone(), f.tab_insert.clone(), f.tab_insert.clone());
        let d_drop = f.deck.clone();
        tabs = tabs.child(
            div()
                .id(eid("tab", pane.0, i))
                .debug_selector(move || format!("tab-{}-{i}", pane.0))
                .flex()
                .flex_shrink(1.)
                .min_w(t.tab_min_w)
                .max_w(t.tab_max_w)
                .h_full()
                .px_3()
                .gap_2()
                .items_center()
                .border_r_1()
                .border_color(t.line)
                .when(active, |d| d.bg(t.bg).text_color(t.fg))
                .when(!active, |d| d.text_color(t.mute))
                .when(active && focused, |d| d.border_t_2().border_color(t.accent))
                .hover(move |s| s.bg(hover))
                .cursor_pointer()
                .tooltip(tip(full))
                .on_click(move |e, _, cx| {
                    let double = e.click_count() == 2;
                    apply_on(&d_click, cx, |l| {
                        l.activate(pane, i)?;
                        if double { l.toggle_maximize(pane) } else { Ok(()) }
                    });
                })
                .on_mouse_down(MouseButton::Middle, move |_, _, cx| apply_on(&d_mid, cx, |l| l.close_tab(pane, i).map(|_| ())))
                .on_mouse_down(MouseButton::Right, {
                    let (deck, window_id) = (f.deck.clone(), f.window);
                    move |e, window, cx| {
                        cx.stop_propagation();
                        let d = deck.read(cx);
                        let items = tab_menu(&deck, pane, i, window_id, d.layout(), &d.book.keep);
                        open_menu(e.position, items, window, cx);
                    }
                })
                .on_drag(DraggedTab { deck: deck_id, pane, index: i, title: title.clone() }, |d, _, _, cx| {
                    cx.new(|_| DragGhost(d.title.clone()))
                })
                .on_drag_move::<DraggedTab>(move |e, window, _| {
                    if !e.bounds.contains(&e.event.position) {
                        return;
                    }
                    let at = if e.event.position.x < e.bounds.center().x { i } else { i + 1 };
                    if moving.get() != Some((pane, at)) {
                        moving.set(Some((pane, at)));
                        window.refresh();
                    }
                })
                .drag_over::<DraggedTab>(move |s, _, _, _| match showing.get() {
                    Some((p, at)) if p == pane && at == i => s.border_l_2().border_color(accent),
                    Some((p, at)) if p == pane && at == i + 1 => s.border_r_2().border_color(accent),
                    _ => s,
                })
                .on_drop(move |tab: &DraggedTab, _, cx| {
                    let at = dropping.take().filter(|(p, _)| *p == pane).map_or(i, |(_, at)| at);
                    if tab.deck == deck_id {
                        let tab = tab.clone();
                        apply_on(&d_drop, cx, |l| l.move_tab_to(tab.pane, tab.index, pane, at));
                    }
                })
                .child(div().flex_1().min_w_0().truncate().debug_selector(|| "tab-title".into()).child(title.clone()))
                .child(
                    div()
                        .id(eid("tab-x", pane.0, i))
                        .flex_none()
                        .px_1()
                        .rounded(t.radius_s)
                        .text_color(t.dim)
                        .hover(move |s| s.bg(line).text_color(fg))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            apply_on(&d_close, cx, |l| l.close_tab(pane, i).map(|_| ()));
                        })
                        .child("×"),
                ),
        );
    }
    strip = strip.child(tabs);

    // Pane buttons.
    let is_detached = f.window != MAIN_WINDOW;
    let (d_col, d_max, d_det) = (f.deck.clone(), f.deck.clone(), f.deck.clone());
    let active = p.active;
    let can_detach = !p.tabs.is_empty() && !(is_detached && p.tabs.len() == 1);
    strip = strip.child(
        div()
            .flex_shrink(1.)
            .min_w_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            .when(!is_detached, |d| {
                d.child(
                    icon_button(("collapse", pane.0), if p.collapsed { "▾" } else { "▴" }, "collapse or expand", t)
                        .on_click(move |_, _, cx| apply_on(&d_col, cx, |l| l.toggle_collapse(pane))),
                )
                .child(
                    icon_button(("maximize", pane.0), if maximized { "◱" } else { "□" }, "maximize or restore", t)
                        .on_click(move |_, _, cx| apply_on(&d_max, cx, |l| l.toggle_maximize(pane))),
                )
            })
            .when(!p.tabs.is_empty(), |d| {
                let deck = f.deck.clone();
                d.child(
                    icon_button(("send", pane.0), "⇢", "send this pane's tabs to another pane", t).on_click(move |_, window, cx| {
                        crate::send_to::open(deck.clone(), SendSource::Pane(pane), window, cx);
                    }),
                )
            })
            .when(can_detach, |d| {
                d.child(
                    icon_button(("detach", pane.0), "⇱", "detach tab to its own window", t)
                        .on_click(move |_, _, cx| apply_on(&d_det, cx, |l| l.detach(pane, active).map(|_| ()))),
                )
            }),
    );

    if p.collapsed {
        return div().size_full().child(strip).into_any_element();
    }

    // Body, with drop zones while something is dragged.
    let d_focus = f.deck.clone();
    let window = f.window;
    let mut body = div()
        .id(("body", pane.0))
        .relative()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            let needs = d_focus.read(cx).layout().window(window).is_ok_and(|w| w.focused != pane);
            if needs {
                apply_on(&d_focus, cx, |l| l.focus(pane));
            }
        })
        .child(match f.views.get(&pane) {
            // Each view is redrawn only when it changes (or on a theme/scale refresh). Esc in
            // an input inside it cancels the edit (the input's own handler runs first and
            // lets it through), then gives the keys back to the view.
            Some(v) => {
                let back = f.view_focus.get(&pane).cloned();
                div()
                    .size_full()
                    .key_context("View")
                    .when_some(back, |d, back| d.on_action(move |_: &devdeck_ui::text_input::Cancel, window, cx| back.focus(window, cx)))
                    .child(v.clone().cached(StyleRefinement::default().size_full()))
                    .into_any_element()
            }
            None => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(t.mute)
                .child(label("Empty pane. Drop a tab here.").debug_selector(|| "empty-message".into()))
                .into_any_element(),
        });
    if f.dragging {
        for (zone, edge) in [
            ("center", None),
            ("left", Some(Edge::Left)),
            ("right", Some(Edge::Right)),
            ("top", Some(Edge::Top)),
            ("bottom", Some(Edge::Bottom)),
        ] {
            body = body.child(drop_zone(pane, zone, edge, f.deck, t));
        }
    }

    div().size_full().flex().flex_col().min_w_0().min_h_0().child(strip).child(body).into_any_element()
}


fn drop_zone(pane: PaneId, name: &'static str, edge: Option<Edge>, deck: &Entity<Deck>, t: &Theme) -> impl IntoElement {
    let zone = div().id((name, pane.0)).absolute();
    let zone = match edge {
        None => zone.left(relative(0.25)).top(relative(0.25)).w(relative(0.5)).h(relative(0.5)),
        Some(Edge::Left) => zone.left_0().top_0().w(relative(0.25)).h_full(),
        Some(Edge::Right) => zone.right_0().top_0().w(relative(0.25)).h_full(),
        Some(Edge::Top) => zone.left(relative(0.25)).top_0().w(relative(0.5)).h(relative(0.25)),
        Some(Edge::Bottom) => zone.left(relative(0.25)).bottom_0().w(relative(0.5)).h(relative(0.25)),
    };
    let drop = match edge {
        None => Drop::Into(pane),
        Some(e) => Drop::Edge(pane, e),
    };
    let (bg, accent) = (t.drop_bg, t.accent);
    let (d_tab, d_pane) = (deck.clone(), deck.clone());
    let deck_id = deck.entity_id();
    let d_item = deck.clone();
    zone.drag_over::<DraggedTab>(move |s, _, _, _| s.bg(bg).border_2().border_color(accent))
        .drag_over::<DraggedPane>(move |s, _, _, _| s.bg(bg).border_2().border_color(accent))
        .drag_over::<DraggedListItem>(move |s, _, _, _| s.bg(bg).border_2().border_color(accent))
        .on_drop(move |item: &DraggedListItem, _, cx| {
            if item.deck == deck_id {
                let view = item.view.clone();
                apply_on(&d_item, cx, |l| sidebar::open_at(l, view, drop));
            }
        })
        .on_drop(move |tab: &DraggedTab, _, cx| {
            if tab.deck == deck_id {
                let tab = tab.clone();
                apply_on(&d_tab, cx, |l| l.move_tab(tab.pane, tab.index, drop));
            }
        })
        .on_drop(move |dragged: &DraggedPane, _, cx| {
            if dragged.deck == deck_id {
                let p = dragged.pane;
                apply_on(&d_pane, cx, |l| l.move_pane(p, drop));
            }
        })
}

#[cfg(test)]
mod tests {
    use super::{frame_color, resize_edge};
    use devdeck_ui::Theme;
    use devdeck_ui::logical as px;
    use gpui::{Decorations, ResizeEdge, Tiling, point, size};

    #[test]
    fn the_frame_shows_unless_the_screen_or_the_compositor_frames_the_window() {
        let t = Theme::devdeck_dark(1.0);
        let floating = Decorations::Client { tiling: Tiling::default() };
        let tiled = Decorations::Client { tiling: Tiling { left: true, top: true, bottom: true, right: false } };
        assert_eq!(frame_color(floating, false, false, true, &t), Some(t.mute), "active: brighter");
        assert_eq!(frame_color(floating, false, false, false, &t), Some(t.line));
        assert_eq!(frame_color(tiled, false, false, true, &t), Some(t.mute), "tiled: framed too");
        assert_eq!(frame_color(Decorations::Client { tiling: Tiling::tiled() }, true, false, true, &t), None, "maximized");
        assert_eq!(frame_color(floating, false, true, true, &t), None, "fullscreen");
        assert_eq!(frame_color(Decorations::Server, false, false, true, &t), None, "the compositor's frame");
    }


    #[test]
    fn edges_are_a_thin_band_of_the_drawn_size() {
        let (s, e, free) = (size(px(1000.), px(600.)), px(6.), Tiling::default());
        assert_eq!(resize_edge(point(px(500.), px(300.)), s, free, e), None, "the middle is content");
        assert_eq!(resize_edge(point(px(997.), px(300.)), s, free, e), Some(ResizeEdge::Right));
        assert_eq!(resize_edge(point(px(2.), px(2.)), s, free, e), Some(ResizeEdge::TopLeft));
        assert_eq!(resize_edge(point(px(500.), px(598.)), s, free, e), Some(ResizeEdge::Bottom));
        // The bug: a maximized 1920 px window measured with its 1000 px restore size made
        // everything past x = 994 an edge.
        assert_eq!(resize_edge(point(px(1500.), px(300.)), size(px(1920.), px(1080.)), free, e), None);
    }

    #[test]
    fn sides_held_by_the_compositor_have_no_edge() {
        let (s, e) = (size(px(1920.), px(1080.)), px(6.));
        // Maximized and fullscreen: GPUI reports every side as tiled.
        for p in [point(px(2.), px(2.)), point(px(1918.), px(540.)), point(px(960.), px(1078.))] {
            assert_eq!(resize_edge(p, s, Tiling::tiled(), e), None, "{p:?}");
        }
        // Snapped to the left half: only the right side resizes, and no corner.
        let left_half = Tiling { top: true, bottom: true, left: true, right: false };
        assert_eq!(resize_edge(point(px(2.), px(540.)), s, left_half, e), None);
        assert_eq!(resize_edge(point(px(1918.), px(540.)), s, left_half, e), Some(ResizeEdge::Right));
        assert_eq!(resize_edge(point(px(1918.), px(2.)), s, left_half, e), Some(ResizeEdge::Right));
    }
}
