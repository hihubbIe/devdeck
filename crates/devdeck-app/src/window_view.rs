//! Root view of one OS window: title bar, pane tree, window edges.
//! The main window shows the active workspace's `Deck` and switches with it; a
//! detached window always belongs to the deck that opened it.
//!
//! Rendering works on a snapshot (`Frame`) taken from the deck at the start of
//! `render`, so drawing never holds a borrow of the deck.

use crate::actions::*;
use crate::deck::{Deck, FLASH, Flash, Held, Keys, SendSource};
use crate::places::{self, Place};
use crate::stats;
use crate::app_settings::{self, Cover};
use crate::notifications::{self, Badge, CloseNotifications, NotificationsView, ServiceNotifications, ToggleNotifications};
use crate::settings_view::SettingsView;
use crate::shell::{self, Shell};
use crate::sidebar::{self, Activity, DraggedListItem, Sidebar, SidebarEscape};
use crate::views;
use devdeck_ui::{APP_ICON, APP_NAME, DragGhost, Empty, Icon, MenuItem, Theme, apply_window_scale, button, button_icon, icon, icon_button, label, labeled_button, logical, open_menu, theme, tip};
use devdeck_core::layout::{Axis, Direction, Drop, Edge, Layout, MAIN_WINDOW, Metrics, Moved, Node, PaneId, Rect, SplitId, ViewRef, WindowId};
use gpui::{
    Animation, AnimationExt, AnyElement, AnyView, App, AppContext, Bounds, Context, CursorStyle, Decorations, Div, DragMoveEvent, Entity, EntityId, ExternalPaths,
    FocusHandle, Focusable, HitboxBehavior, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Point, Render, ResizeEdge, SharedString, Stateful, StyleRefinement, Size, StatefulInteractiveElement, Styled, Subscription, WeakEntity,
    Tiling, WeakFocusHandle, Window, WindowBounds, WindowDecorations, WindowOptions, canvas, div, img, point, prelude::*,
    size,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

/// A control in the title bar: pressing it doesn't start a window move, which hands the
/// pointer to the compositor and loses the release, and with it the click.
fn bar_control(control: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
    control.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

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

/// The sidebar's right edge being dragged (R4.9).
#[derive(Clone)]
pub struct DraggedSidebarEdge;

/// The narrowest the sidebar gets, at UI scale 1.
pub const SIDEBAR_MIN: f32 = 200.;

/// The sidebar's drawn width: `saved` (at UI scale 1; else the theme's), kept between
/// [`SIDEBAR_MIN`] and half the window, without changing what's saved.
pub fn sidebar_width(saved: Option<f32>, window_w: Pixels, t: &Theme) -> Pixels {
    let (min, w) = (t.size(SIDEBAR_MIN), saved.map_or(t.sidebar_w, |w| t.size(w)));
    let max = if window_w / 2. > min { window_w / 2. } else { min };
    if w < min { min } else if w > max { max } else { w }
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
    /// Main window: the notification center, made when first opened.
    notifications: Option<Entity<NotificationsView>>,
    /// The bell's badge last drawn.
    badge: Badge,
    /// Main window: where focus was in each workspace, restored on switching back, and
    /// the pane that had the keys (kept for it by quiet changes).
    remembered: HashMap<EntityId, (WeakFocusHandle, Option<Held>)>,
    /// While a tab is dragged over a tab strip: the pane and the place it would land at.
    tab_insert: Rc<Cell<Option<(PaneId, usize)>>>,
    /// While a tab, a pane or a list item is dragged over the panes: the drop under the
    /// pointer and the rectangle it lands in (R3.13).
    drop_hint: Rc<Cell<Option<(Drop, Rect)>>>,
    /// Where focus was when a cover (Settings, the notification center) opened, and the
    /// pane that had the keys.
    before_cover: Option<(WeakFocusHandle, Option<Held>)>,
    /// The deck, and its runner (for the run tabs' icon colours).
    deck_sub: Vec<Subscription>,
    /// The run tabs' icon colours last drawn.
    run_tints: Vec<(ViewRef, views::Tint)>,
    /// Main window: the active workspace's sidebar.
    sidebar: Option<(Entity<Sidebar>, Subscription)>,
    /// Top-left of the pane area (keyboard-opened menus are placed from it).
    panes_origin: Rc<Cell<Point<Pixels>>>,
    /// Focused pane and its active view when keys last moved to a view.
    view_target: Option<(PaneId, Option<devdeck_core::layout::ViewRef>)>,
    _observe: Vec<Subscription>,
}

/// Where the user's keys are in `deck`, in each OS window: the pane they go to now, and
/// the one they go back to when Settings closes or the workspace shows again
/// ([`Deck::apply_quietly`]). Call it outside any update of `deck` or of a window showing
/// it; `None` when such a window can't be asked (it's being updated).
pub fn keys(deck: &Entity<Deck>, cx: &mut App) -> Option<Keys> {
    let mut out = Keys::new();
    for handle in cx.windows() {
        let Some(h) = handle.downcast::<WindowView>() else { continue };
        let w = h.read(cx).ok()?;
        if w.deck != *deck {
            out.extend(w.remembered.get(&deck.entity_id()).and_then(|(_, held)| held.clone()));
            continue;
        }
        let held = h.update(cx, |w, window, cx| w.keys_view(window, cx).or_else(|| w.before_cover.as_ref().and_then(|(_, h)| h.clone()))).ok()?;
        out.extend(held);
    }
    Some(out)
}

/// Options shared by every devdeck window; `size` in logical pixels, the windowed size
/// (restoring a `maximized` one gives it back, R9.8).
pub fn window_options((w, h): (f32, f32), maximized: bool, cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(logical(w), logical(h)), cx);
    WindowOptions {
        window_bounds: Some(if maximized { WindowBounds::Maximized(bounds) } else { WindowBounds::Windowed(bounds) }),
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
    /// Every tab's title, as its view says it (`Deck::title`): its tooltip.
    titles: &'a HashMap<ViewRef, SharedString>,
    /// Every tab's icon and short name: what it shows.
    labels: &'a HashMap<ViewRef, views::TabLabel>,
    /// What each pane's active view offers now (R12.4), for its header.
    actions: &'a HashMap<PaneId, Vec<views::ViewAction>>,
    t: &'a Theme,
    tab_insert: &'a Rc<Cell<Option<(PaneId, usize)>>>,
    /// The tab flashing after a reveal, if one is.
    flash: Option<&'a Flash>,
    /// The flash is drawn still, not fading ([`App::reduce_motion`]).
    still: bool,
}

impl WindowView {
    /// The main window: shows the shell's active workspace. Closing it asks to quit.
    pub fn main(shell: Entity<Shell>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let deck = shell.read(cx).active_deck();
        let mut observe = vec![
            // Settings or the notification center cover the panes: their views are off
            // screen meanwhile.
            cx.observe(&app_settings::settings(cx), |this, _, cx| {
                let open = app_settings::is_covered(cx);
                this.deck.update(cx, |d, cx| d.set_covered(open, cx));
                cx.notify();
            }),
            cx.observe_in(&shell, window, |this, shell, window, cx| this.follow_shell(&shell, window, cx)),
        ];
        let size_shell = shell.downgrade();
        observe.push(cx.observe_window_bounds(window, move |_, window, cx| {
            let (size, maximized) = (free_size(window), window.is_maximized());
            let _ = size_shell.update(cx, |s, _| {
                s.main_size = size.or(s.main_size);
                s.main_maximized = maximized;
            });
        }));
        window.on_window_should_close(cx, |_, cx| {
            cx.defer(shell::request_quit);
            false
        });
        window.set_window_title(&format!("{APP_NAME} · {}", shell.read(cx).active_name()));
        notifications::install(cx);
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
        window.set_window_title(&format!("{APP_NAME} · {} · detached", deck.read(cx).workspace));
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
        // The bell redraws when its badge changes, not on every toast.
        let store = devdeck_ui::toasts(cx);
        observe.push(cx.observe(&store, |this: &mut Self, _, cx| {
            let b = notifications::badge(cx);
            if b != this.badge {
                this.badge = b;
                cx.notify();
            }
        }));
        let badge = notifications::badge(cx);
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
            notifications: None,
            badge,
            remembered: HashMap::new(),
            tab_insert: Rc::default(),
            drop_hint: Rc::default(),
            before_cover: None,
            deck_sub,
            run_tints: Vec::new(),
            sidebar: None,
            view_target: None,
            panes_origin: Rc::new(Cell::new(Point::default())),
            _observe: observe,
        };
        this.sync_view_focus(window, cx);
        this
    }

    fn observe_deck(deck: &Entity<Deck>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Subscription> {
        let mut subs = vec![cx.observe_in(deck, window, |this, _, window, cx| {
            this.sync_view_focus(window, cx);
            cx.notify();
        })];
        // A run starting, stopping or failing recolours its tab's icon; output doesn't
        // redraw the window.
        if let Some(services) = deck.read(cx).services() {
            let runner = services.read(cx).runner.clone();
            subs.push(cx.observe(&runner, move |this, _, cx| {
                let d = this.deck.read(cx);
                let Some(s) = d.services() else { return };
                let tints = views::run_tints(d.layout(), s.read(cx), cx);
                if tints != this.run_tints {
                    this.run_tints = tints;
                    cx.notify();
                }
            }));
        }
        subs
    }

    /// The pane this window's keys go to, if they're in the pane area: the one whose view
    /// has them, else the focused pane (its keys act on it).
    fn keys_view(&self, window: &Window, cx: &App) -> Option<Held> {
        let d = self.deck.read(cx);
        let l = d.layout();
        let in_view = l.panes_in(self.id).ok()?.into_iter().find(|p| d.view_focus(*p).is_some_and(|h| h.contains_focused(window, cx)));
        let pane = match in_view {
            Some(p) => p,
            None if self.panes_focus.contains_focused(window, cx) => l.window(self.id).ok()?.focused,
            None => return None,
        };
        Some(Held { window: self.id, pane, view: l.pane(pane).ok()?.active_view().cloned() })
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
            if !app_settings::is_covered(cx)
                && let Some(f) = window.focused(cx)
            {
                let held = self.keys_view(window, cx);
                self.remembered.insert(self.deck.entity_id(), (f.downgrade(), held));
            }
            self.deck_sub = Self::observe_deck(&deck, window, cx);
            let open = app_settings::is_covered(cx);
            deck.update(cx, |d, cx| d.set_covered(open, cx));
            self.deck = deck;
            self.view_target = None;
            let sidebar = shell.read(cx).active_sidebar();
            self.sidebar = Some((sidebar.clone(), cx.observe(&sidebar, |_, _, cx| cx.notify())));
            window.set_window_title(&format!("{APP_NAME} · {}", shell.read(cx).active_name()));
            if !app_settings::is_covered(cx) {
                match self.remembered.get(&self.deck.entity_id()).and_then(|(h, _)| h.upgrade()) {
                    Some(h) => h.focus(window, cx),
                    None => self.focus_panes(window, cx),
                }
            }
        }
        cx.notify();
    }

    /// Opens or closes the full-window Settings view (main window only).
    fn set_settings_open(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        match (open, app_settings::cover(cx)) {
            (true, _) => self.set_cover(Cover::Settings, window, cx),
            (false, Cover::Settings) => self.set_cover(Cover::None, window, cx),
            (false, _) => {}
        }
    }

    /// Shows `cover` over the panes (main window only), one at a time: focus moves into
    /// it, and back to where it was when nothing covers them any more.
    fn set_cover(&mut self, cover: Cover, window: &mut Window, cx: &mut Context<Self>) {
        let was = app_settings::cover(cx);
        if !self.is_main() || cover == was {
            return;
        }
        if was == Cover::None {
            self.before_cover = window.focused(cx).map(|f| (f.downgrade(), self.keys_view(window, cx)));
        }
        app_settings::set_cover(cover, cx);
        match cover {
            Cover::None => match self.before_cover.take().and_then(|(h, _)| h.upgrade()) {
                Some(h) => h.focus(window, cx),
                None => self.focus_panes(window, cx),
            },
            Cover::Settings => {
                let view = self.settings.get_or_insert_with(|| cx.new(SettingsView::new)).clone();
                view.read(cx).focus_handle(cx).focus(window, cx);
            }
            Cover::Notifications => {
                let view = self.notifications.get_or_insert_with(|| cx.new(NotificationsView::new)).clone();
                view.update(cx, |v, cx| v.show(None, cx));
                view.read(cx).focus_handle(cx).focus(window, cx);
            }
        }
        cx.notify();
    }

    /// Opens the notification center (on notification `on`): here in the main window,
    /// else in the main window brought forward.
    pub fn open_notifications(&mut self, on: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_main() {
            notifications::open_center(on, cx);
            return;
        }
        self.set_cover(Cover::Notifications, window, cx);
        if let Some(view) = &self.notifications {
            view.update(cx, |v, cx| v.show(on, cx));
        }
    }

    /// The bell, or its key: the center opens, or closes when it's open here.
    fn toggle_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_main() && app_settings::cover(cx) == Cover::Notifications {
            self.set_cover(Cover::None, window, cx);
        } else {
            self.open_notifications(None, window, cx);
        }
    }

    /// Nothing covers the panes any more (Settings or the center closes).
    pub fn uncover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_cover(Cover::None, window, cx);
    }

    /// What covers the panes closes for something just opened there: the keys go to the
    /// focused pane (showing it), not back to where they were before the cover.
    pub fn uncover_to_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if app_settings::is_covered(cx) {
            self.before_cover = None;
            self.set_cover(Cover::None, window, cx);
        }
    }

    /// A view of this window was revealed ([`Shell::reveal`]): the window comes to the
    /// front (where the window system refuses, as Wayland does without a recent input, it
    /// asks for attention), what covers the panes makes way, the keys go to the view.
    pub fn bring_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        if !window.is_window_active() {
            window.request_attention();
        }
        self.uncover_to_panes(window, cx);
        self.focus_panes(window, cx);
    }

    /// The notification center (main window, once opened).
    pub fn notifications_view(&self) -> Option<Entity<NotificationsView>> {
        self.notifications.clone()
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
            let rect = d.layout().pane_rects(self.id, size, metrics(&t)).ok().and_then(|r| r.into_iter().find(|(q, _)| *q == p)).map(|(_, r)| r);
            (rect, d.layout().pane(p).map(|p| p.active).unwrap_or(0))
        };
        let origin = self.panes_origin.get();
        let at = rect.map_or(origin, |r| point(origin.x + logical(r.x) + t.size(24.), origin.y + logical(r.y) + t.tab_h));
        let d = self.deck.read(cx);
        // An empty pane has no tab: its menu is the pane's (R3.12).
        let items = if d.layout().pane(p).is_ok_and(|pane| pane.tabs.is_empty()) {
            pane_menu(&self.deck, p, d.layout(), cx)
        } else {
            tab_menu(&self.deck, p, active, self.id, d.layout(), cx)
        };
        open_menu(at, items, window, cx);
    }

    /// ctrl+k: the palette opens in the main window (brought forward from a detached one).
    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_main() {
            if !app_settings::is_covered(cx) {
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

    /// The sidebar's drawn width in this window.
    fn sidebar_w(&self, window: &Window, t: &Theme, cx: &App) -> Pixels {
        let saved = self.shell.as_ref().and_then(WeakEntity::upgrade).and_then(|s| s.read(cx).sidebar_w);
        sidebar_width(saved, window.viewport_size().width, t)
    }

    /// Shows an activity and focuses its list (ctrl+1…6, activity bar).
    fn show_activity(&mut self, activity: Activity, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sidebar) = self.active_sidebar() else { return };
        if app_settings::is_covered(cx) {
            return;
        }
        if sidebar.update(cx, |s, cx| s.show(activity, cx)) {
            sidebar.read(cx).list_focus.clone().focus(window, cx);
        }
    }

    /// Shows or hides the sidebar; hiding it while it has focus sends focus to the panes.
    fn set_sidebar_visible(&mut self, visible: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sidebar) = self.active_sidebar() else { return };
        if app_settings::is_covered(cx) {
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
        if sidebar.read(cx).envs(cx).is_empty() || app_settings::is_covered(cx) {
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

    /// The pane area's size, as the neighbour keys and the tab menu see it.
    pub fn pane_area(&self, window: &Window, cx: &App) -> (f32, f32) {
        self.body_size(window, theme(cx), cx)
    }

    /// The pane area's size: the window body minus the activity bar, the sidebar and
    /// its edge.
    fn body_size(&self, window: &Window, t: &Theme, cx: &App) -> (f32, f32) {
        let s = window.viewport_size();
        let mut w = s.width;
        if let Some(sidebar) = self.active_sidebar() {
            w -= t.activity_w;
            if sidebar.read(cx).state.visible {
                w -= self.sidebar_w(window, t, cx) + t.divider_grab;
            }
        }
        (f32::from(w), f32::from(s.height - t.title_h))
    }

    fn focus_dir(&mut self, dir: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.focused_pane(cx) else { return };
        let t = theme(cx);
        let (size, m) = (self.body_size(window, t, cx), metrics(t));
        self.apply(cx, |l| {
            if let Some(n) = l.neighbor(p, dir, size, m)? {
                l.focus(n)?;
            }
            Ok(())
        });
    }

    fn move_tab_dir(&mut self, dir: Direction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = self.focused_pane(cx) else { return };
        let t = theme(cx);
        let (size, m) = (self.body_size(window, t, cx), metrics(t));
        self.apply(cx, |l| {
            if l.pane(p)?.tabs.is_empty() {
                return Ok(());
            }
            let idx = l.pane(p)?.active;
            match l.neighbor(p, dir, size, m)? {
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

    /// The focused tab beside everything in the window, on `edge` (R3.13 by keyboard).
    fn move_tab_to_edge(&mut self, edge: Edge, cx: &mut Context<Self>) {
        let id = self.id;
        self.with_focused(cx, |l, p| {
            // A detached window is a single pane: no edges (R3.8).
            if l.pane(p)?.tabs.is_empty() || l.is_detached(id) {
                return Ok(());
            }
            let (i, drop) = (l.pane(p)?.active, l.window_edge(id, edge)?);
            l.move_tab(p, i, drop)
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
            d.open_terminal(&dir, Place::Pane(pane), cx);
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

/// How the panes are drawn, for the layout's geometry: dividers' grab area, a collapsed
/// pane's rail (in a row) and header (in a column).
fn metrics(t: &Theme) -> Metrics {
    Metrics { divider: f32::from(t.divider_grab), rail: f32::from(t.rail_w), header: f32::from(t.tab_h) }
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
        let cover = if is_main { app_settings::cover(cx) } else { Cover::None };
        let settings_open = cover == Cover::Settings;
        let covered = cover != Cover::None;
        if settings_open && self.settings.is_none() {
            self.settings = Some(cx.new(SettingsView::new));
        }
        if cover == Cover::Notifications && self.notifications.is_none() {
            self.notifications = Some(cx.new(NotificationsView::new));
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
        let (titles, labels, actions) = {
            let d = self.deck.read(cx);
            let titles: HashMap<ViewRef, SharedString> = layout.panes.values().flat_map(|p| p.tabs.iter()).map(|r| (r.clone(), d.title(r, cx).into())).collect();
            let services = d.services();
            let s = services.as_ref().map(|e| e.read(cx));
            let labels: HashMap<ViewRef, views::TabLabel> = layout.panes.values().flat_map(|p| p.tabs.iter()).map(|r| (r.clone(), views::tab_label(r, s, cx))).collect();
            let actions: HashMap<PaneId, Vec<views::ViewAction>> =
                layout.panes.iter().filter_map(|(id, p)| p.active_view().map(|r| (*id, d.view_actions(r, cx)))).filter(|(_, a)| !a.is_empty()).collect();
            (titles, labels, actions)
        };
        let flash = self.deck.read(cx).flash.clone();
        let frame = Frame {
            flash: flash.as_ref(),
            still: cx.reduce_motion(),
            layout: &layout,
            numbers: &numbers,
            view_focus: &view_focus,
            views: &views,
            window: self.id,
            deck: &self.deck,
            titles: &titles,
            labels: &labels,
            actions: &actions,
            t: &t,
            tab_insert: &self.tab_insert,
        };
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
            // The icon alone (R9.7): the name beside it read as a menu.
            .child(
                div()
                    .id("app-icon")
                    .flex()
                    .items_center()
                    .tooltip(tip(APP_NAME))
                    .child(img(APP_ICON).flex_none().size(t.size(23.))),
            )
            .when_some(menu_shell, |d, shell| {
                d.child(
                    bar_control(button("workspace", ws_name.clone(), &t))
                        .child(button_icon(Icon::Open, &t))
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
                let color = if env.protected { t.err } else { t.env };
                d.child(
                    bar_control(button("env", format!("@{}", env.name), &t))
                        // Protected: changes there ask first.
                        .when(env.protected, |b| b.child(icon(Icon::Protected, color, &t)))
                        .child(icon(Icon::Open, color, &t))
                        .text_color(color)
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
                let current = deck.read(cx).book.current_name().to_string();
                d.child(
                    bar_control(button("layout", current, &t))
                        .child(button_icon(Icon::Open, &t))
                        .debug_selector(|| "layout-button".into())
                        .tooltip(tip("pulses"))
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
                    bar_control(button("search", "search", &t))
                        .debug_selector(|| "search-button".into())
                        .on_click(|_, window, cx| crate::palette::open(window, cx)),
                )
                .child(
                    bar_control(button("keys", "?", &t))
                        .tooltip(tip("keyboard shortcuts"))
                        .on_click(|_, window, cx| crate::cheat_sheet::open(window, cx)),
                )
            })
            .child(
                bar_control(notifications::bell(self.badge, &t))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_notifications(window, cx))),
            )
            .when(is_main, |d| {
                d.child(
                    bar_control(labeled_button("settings", Icon::Settings, "settings", &t))
                        .debug_selector(|| "settings-button".into())
                        // The check's room is kept closed too, so opening never widens it (R9.10).
                        .child(
                            div()
                                .debug_selector(|| "settings-check".into())
                                .size(t.icon)
                                .flex_none()
                                .when(settings_open, |d| d.child(icon(Icon::Check, t.accent, &t))),
                        )
                        .tooltip(tip("settings"))
                        .on_click(cx.listener(move |this, _, window, cx| this.set_settings_open(!settings_open, window, cx))),
                )
            })
            .when(!is_main, |d| {
                d.child(
                    bar_control(icon_button("reattach", Icon::Reattach, "send back to the main window", &t))
                        .on_click(cx.listener(|this, _, _, cx| this.reattach(cx))),
                )
            })
            .child(bar_control(icon_button("min", Icon::WindowMinimize, "minimize", &t)).on_click(|_, window, _| window.minimize_window()))
            .child(
                bar_control(icon_button("max", if window.is_maximized() { Icon::WindowRestore } else { Icon::WindowMaximize }, "maximize or restore", &t))
                    .on_click(|_, window, _| window.zoom_window()),
            )
            .child(bar_control(icon_button("close", Icon::Close, "close", &t)).on_click(move |_, _, cx| {
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
            .on_action(cx.listener(|this, _: &MoveTabToLeftEdge, _, cx| this.move_tab_to_edge(Edge::Left, cx)))
            .on_action(cx.listener(|this, _: &MoveTabToRightEdge, _, cx| this.move_tab_to_edge(Edge::Right, cx)))
            .on_action(cx.listener(|this, _: &MoveTabToTopEdge, _, cx| this.move_tab_to_edge(Edge::Top, cx)))
            .on_action(cx.listener(|this, _: &MoveTabToBottomEdge, _, cx| this.move_tab_to_edge(Edge::Bottom, cx)))
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
            .child({
                // The drop layer is painted first, under the panes: tab strips and headers
                // take their own drops (GPUI gives a drop to the topmost taker), the rest
                // falls to it. The preview has no listeners, so no hitbox.
                let dragging = cx.has_active_drag();
                if !dragging {
                    self.drop_hint.set(None);
                }
                let hint = self.drop_hint.get().filter(|_| dragging);
                div()
                    .id(("ws", deck_id.as_u64()))
                    .debug_selector(|| "panes".into())
                    .relative()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .when(dragging, |d| d.child(drop_layer(&self.deck, self.id, &self.drop_hint, &t)))
                    .child(tree)
                    .when_some(hint, |d, (_, r)| {
                        d.child(
                            div()
                                .debug_selector(|| "drop-preview".into())
                                .absolute()
                                .left(logical(r.x))
                                .top(logical(r.y))
                                .w(logical(r.w))
                                .h(logical(r.h))
                                .bg(t.drop_bg)
                                .border_2()
                                .border_color(t.accent),
                        )
                    })
            })
            .child({
                let origin = self.panes_origin.clone();
                canvas(move |bounds, _, _| origin.set(bounds.origin), |_, _, _, _| {}).absolute().size_full()
            });
        let body = match (cover, &self.settings, &self.notifications) {
            (Cover::Settings, Some(view), _) => div().flex_1().min_h_0().min_w_0().flex().child(view.clone()).into_any_element(),
            (Cover::Notifications, _, Some(view)) => div().flex_1().min_h_0().min_w_0().flex().child(view.clone()).into_any_element(),
            _ => match self.active_sidebar() {
                Some(sidebar) => {
                    let visible = sidebar.read(cx).state.visible;
                    let width = self.sidebar_w(window, &t, cx);
                    let (shell, drag_shell) = (self.shell.clone(), self.shell.clone());
                    let (activity_w, grab, scale) = (t.activity_w, t.divider_grab, t.scale);
                    div()
                        .id("body")
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .flex()
                        .child(activity_bar(&sidebar, &t, cx))
                        .when(visible, |d| {
                            // Redrawn only when the sidebar itself changes (or its width),
                            // not on every frame of a streaming view next to it.
                            d.child(sidebar.clone().cached(StyleRefinement::default().flex_none().w(width).h_full()))
                                .child(sidebar_edge(shell, &t))
                        })
                        .on_drag_move::<DraggedSidebarEdge>(move |e: &DragMoveEvent<DraggedSidebarEdge>, window, cx| {
                            let Some(shell) = drag_shell.as_ref().and_then(WeakEntity::upgrade) else { return };
                            let wanted = e.event.position.x - e.bounds.origin.x - activity_w - grab / 2.;
                            let t = theme(cx).clone();
                            let w = sidebar_width(Some(f32::from(wanted) / scale), window.viewport_size().width, &t);
                            shell.update(cx, |s, _| s.sidebar_w = Some(f32::from(w) / scale));
                            window.refresh();
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
                .on_action(cx.listener(|this, _: &CloseNotifications, w, cx| {
                    if app_settings::cover(cx) == Cover::Notifications {
                        this.set_cover(Cover::None, w, cx);
                    }
                }))
            })
            .on_action(cx.listener(|this, _: &ToggleNotifications, w, cx| this.toggle_notifications(w, cx)))
            .on_action(|_: &ServiceNotifications, w, cx| notifications::open_subscriptions(w, cx))
            .when(!is_main, |d| d.on_action(cx.listener(|this, _: &Reattach, _, cx| this.reattach(cx))))
            .when(is_main && !covered, |d| {
                d.on_action(cx.listener(|this, _: &ShowApps, w, cx| this.show_activity(Activity::Apps, w, cx)))
                    .on_action(cx.listener(|this, _: &ShowCluster, w, cx| this.show_activity(Activity::Cluster, w, cx)))
                    .on_action(cx.listener(|this, _: &ShowPorts, w, cx| this.show_activity(Activity::Ports, w, cx)))
                    .on_action(cx.listener(|this, _: &ShowLayouts, w, cx| this.show_activity(Activity::Layouts, w, cx)))
                    .on_action(cx.listener(|this, _: &ShowTerminals, w, cx| this.show_activity(Activity::Terminals, w, cx)))
                    .on_action(cx.listener(|this, _: &ShowNotes, w, cx| this.show_activity(Activity::Notes, w, cx)))
                    .on_action(cx.listener(|this, _: &NewLayout, w, cx| crate::layouts::open_new(this.deck.clone(), w, cx)))
                    .on_action(cx.listener(|this, _: &SaveLayoutAs, w, cx| crate::layouts::open_save_as(this.deck.clone(), w, cx)))
                    .on_action(cx.listener(|this, _: &ToggleSidebar, w, cx| {
                        let visible = this.active_sidebar().is_some_and(|s| s.read(cx).state.visible);
                        this.set_sidebar_visible(!visible, w, cx);
                    }))
                    .on_action(cx.listener(|this, _: &OpenEnvMenu, w, cx| this.open_env_menu(w, cx)))
                    // Esc with an empty filter: back to the panes.
                    .on_action(cx.listener(|this, _: &SidebarEscape, w, cx| this.focus_panes(w, cx)))
            })
            .when(!covered, |d| d.on_action(cx.listener(|this, _: &ContextMenu, w, cx| this.open_context_menu(w, cx))))
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
                .relative()
                .w(t.activity_w)
                .h(t.activity_w)
                .flex()
                .items_center()
                .justify_center()
                // The current one: a line drawn over its left edge, not a border, so its icon
                // doesn't move (R9.10).
                .when(on, |d| d.child(div().absolute().top_0().left_0().h_full().w(t.hairline * 2.).bg(t.accent)))
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
                .child(icon(a.icon(), if on { t.hi } else { t.mute }, t).size(t.size(18.)).debug_selector(move || format!("activity-icon-{}", a.id()))),
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

/// The sidebar's right edge: dragged, it sets the width; a double click goes back to
/// the theme's.
fn sidebar_edge(shell: Option<WeakEntity<Shell>>, t: &Theme) -> impl IntoElement {
    div()
        .id("sidebar-edge")
        .debug_selector(|| "sidebar-edge".into())
        .flex_none()
        .w(t.divider_grab)
        .h_full()
        .flex()
        .justify_center()
        .cursor_col_resize()
        .child(div().w(t.hairline).h_full().bg(t.line))
        .on_drag(DraggedSidebarEdge, |_, _, _, cx| cx.new(|_| Empty))
        .on_mouse_up(MouseButton::Left, move |e, window, cx| {
            if e.click_count == 2
                && let Some(shell) = shell.as_ref().and_then(WeakEntity::upgrade)
            {
                shell.update(cx, |s, _| s.sidebar_w = None);
                window.refresh();
            }
        })
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

/// Runs a view's action (R12.4) on that pane's view, whichever pane has the focus: the
/// pane (and tab `idx`, when given) comes forward and focused, and the view runs it
/// itself (a tab just brought forward isn't drawn yet, so a dispatched action would
/// miss it).
fn run_view_action(deck: &Entity<Deck>, pane: PaneId, idx: Option<usize>, id: &'static str, window: &mut Window, cx: &mut App) {
    apply_on(deck, cx, |l| {
        if let Some(i) = idx {
            l.activate(pane, i)?;
        }
        l.focus(pane)
    });
    let d = deck.read(cx);
    let view = d.layout().pane(pane).ok().and_then(|p| p.active_view().cloned());
    if let Some(f) = d.view_focus(pane) {
        f.focus(window, cx);
    }
    if let Some(l) = view.and_then(|r| deck.read(cx).lively(&r)) {
        l.run(id, window, cx);
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
fn tab_menu(deck: &Entity<Deck>, pane: PaneId, idx: usize, window: WindowId, layout: &Layout, cx: &App) -> Vec<MenuItem> {
    let Some(view) = layout.pane(pane).ok().and_then(|p| p.tabs.get(idx).cloned()) else { return Vec::new() };
    let alone = layout.is_detached(window) && layout.pane(pane).is_ok_and(|p| p.tabs.len() == 1);
    let maximized = layout.window(window).is_ok_and(|w| w.maximized == Some(pane));
    let d = |deck: &Entity<Deck>| deck.clone();
    let (d1, d2, d3, d4, d5, d6) = (d(deck), d(deck), d(deck), d(deck), d(deck), d(deck));
    let (v1, v2) = (view.clone(), view.clone());
    let terminal = open_terminal_item(deck, pane, window, Some(&view), cx);
    // What the view offers now (R12.4), first.
    let mut items: Vec<MenuItem> = deck
        .read(cx)
        .view_actions(&view, cx)
        .into_iter()
        .filter_map(|a| {
            let spec = crate::registry::by_id(a.id)?;
            let (deck, id) = (deck.clone(), a.id);
            Some(MenuItem::new(spec.label, move |window, cx| run_view_action(&deck, pane, Some(idx), id, window, cx)).toggle(a.on))
        })
        .collect();
    if !items.is_empty() {
        items.push(MenuItem::separator());
    }
    let mut detach = MenuItem::new("Detach to a window", move |_, cx| apply_on(&d3, cx, |l| l.detach(pane, idx).map(|_| ())));
    if alone {
        detach = detach.disabled("already alone in its window");
    }
    items.extend([
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
        terminal,
    ]);
    items
}

/// "Open terminal here": a shell in the folder of `view` (the pane's active tab when
/// `None`), in `pane` on a click, elsewhere from its submenu.
fn open_terminal_item(deck: &Entity<Deck>, pane: PaneId, window: WindowId, view: Option<&ViewRef>, cx: &App) -> MenuItem {
    let d = deck.read(cx);
    let folder = match view {
        Some(v) => d.folder_of(Some(v), cx),
        None => d.terminal_folder(Some(pane), cx),
    };
    places::item("Open terminal here", Place::Pane(pane), window, deck, None, cx, move |d, place, cx| d.open_terminal(&folder, place, cx))
}

/// What a tab does by mouse, on the strip and as a header's title (R3.12): a click
/// activates it (two maximize), middle-click closes, right-click opens its menu, it drags.
fn tab_handlers(el: Stateful<Div>, f: &Frame, pane: PaneId, i: usize, title: &SharedString) -> Stateful<Div> {
    let (d_click, d_mid, d_menu) = (f.deck.clone(), f.deck.clone(), f.deck.clone());
    let (deck_id, window_id) = (f.deck.entity_id(), f.window);
    el.on_click(move |e, _, cx| {
        let double = e.click_count() == 2;
        apply_on(&d_click, cx, |l| {
            l.activate(pane, i)?;
            if double { l.toggle_maximize(pane) } else { Ok(()) }
        });
    })
    .on_mouse_down(MouseButton::Middle, move |_, _, cx| apply_on(&d_mid, cx, |l| l.close_tab(pane, i).map(|_| ())))
    .on_mouse_down(MouseButton::Right, move |e, window, cx| {
        cx.stop_propagation();
        let d = d_menu.read(cx);
        let items = tab_menu(&d_menu, pane, i, window_id, d.layout(), cx);
        open_menu(e.position, items, window, cx);
    })
    .on_drag(DraggedTab { deck: deck_id, pane, index: i, title: title.clone() }, |d, _, _, cx| cx.new(|_| DragGhost(d.title.clone())))
}

/// The menu of a pane (right-click on its tab strip or grip, or on an empty pane's header).
fn pane_menu(deck: &Entity<Deck>, pane: PaneId, layout: &Layout, cx: &App) -> Vec<MenuItem> {
    let tabs = layout.pane(pane).map(|p| p.tabs.len()).unwrap_or(0);
    let d = |deck: &Entity<Deck>| deck.clone();
    let (d1, d2, d3, d4, d5, d6) = (d(deck), d(deck), d(deck), d(deck), d(deck), d(deck));
    let split = |item: MenuItem| if tabs < 2 { item.disabled("needs two tabs") } else { item };
    let some = |item: MenuItem| if tabs == 0 { item.disabled("no tabs") } else { item };
    vec![
        split(MenuItem::new("Split right", move |_, cx| apply_on(&d1, cx, |l| split_active(l, pane, Edge::Right)))),
        split(MenuItem::new("Split down", move |_, cx| apply_on(&d2, cx, |l| split_active(l, pane, Edge::Bottom)))),
        MenuItem::separator(),
        MenuItem::new("Collapse", move |_, cx| apply_on(&d3, cx, |l| l.toggle_collapse(pane))),
        MenuItem::new("Maximize", move |_, cx| apply_on(&d4, cx, |l| l.toggle_maximize(pane))),
        some(MenuItem::new("Send pane to…", move |window, cx| crate::send_to::open(d5.clone(), SendSource::Pane(pane), window, cx))),
        MenuItem::separator(),
        open_terminal_item(deck, pane, layout.window_of(pane).unwrap_or(MAIN_WINDOW), None, cx),
        MenuItem::separator(),
        some(MenuItem::new("Close pane", move |_, cx| {
            apply_on(&d6, cx, |l| {
                let views = l.pane(pane)?.tabs.clone();
                for v in views {
                    l.close_view(&v)?;
                }
                Ok(())
            })
        })),
    ]
}

fn render_pane_inner(f: &Frame, pane: PaneId, parent: Option<Axis>) -> AnyElement {
    let Ok(p) = f.layout.pane(pane) else { return div().into_any_element() };
    let t = f.t;
    let win = f.layout.window(f.window).ok();
    let focused = win.is_some_and(|w| w.focused == pane);
    let maximized = win.is_some_and(|w| w.maximized == Some(pane));
    let titles: Vec<SharedString> = p.tabs.iter().map(|r| f.titles.get(r).cloned().unwrap_or_else(|| views::title(r).into())).collect();
    let labels: Vec<views::TabLabel> = p
        .tabs
        .iter()
        .map(|r| f.labels.get(r).cloned().unwrap_or_else(|| views::TabLabel { icon: Icon::File, tint: views::Tint::Plain, name: views::title(r), detail: None, protected: false }))
        .collect();
    let deck_id = f.deck.entity_id();
    // A revealed view flashes: its tab and its pane's border (it's the active tab).
    let flash = f.flash.filter(|fl| p.active_view() == Some(&fl.view)).map(|fl| fl.n);

    // Collapsed in a row split: a narrow rail of monograms (GPUI can't rotate text).
    if p.collapsed && parent == Some(Axis::Row) {
        let mut rail = div().id(("rail", pane.0)).size_full().flex().flex_col().items_center().gap_1().pt_1().bg(t.bar);
        for (i, title) in titles.iter().enumerate() {
            let deck = f.deck.clone();
            let full = title.clone();
            let hover = t.hover;
            let color = if i == p.active { t.fg } else { t.mute };
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
                    .child(icon(labels[i].icon, views::tint_color(&labels[i], color, t), t)),
            );
        }
        return rail.into_any_element();
    }

    // One tab or none: a header naming the view, its title being the tab (R3.12); more:
    // the tab strip. Either is a drop target for tabs (move into this pane).
    let header = p.tabs.len() <= 1;
    let one = p.tabs.len() == 1;
    let drop_bg = t.drop_bg;
    let strip_deck = f.deck.clone();
    let mut strip = div()
        .id(("strip", pane.0))
        .when(header, |d| d.debug_selector(move || format!("pane-header-{}", pane.0)))
        .relative()
        .flex_none()
        .h(t.tab_h)
        .w_full()
        .min_w_0()
        .flex()
        .bg(t.bar)
        .border_b_1()
        .border_color(t.line)
        .drag_over::<DraggedTab>(move |s, _, _, _| s.bg(drop_bg))
        .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(drop_bg))
        .on_drop({
            let deck = f.deck.clone();
            move |paths: &ExternalPaths, _, cx| crate::drops::open_files(&deck, paths.paths(), Drop::Into(pane), cx)
        })
        .on_mouse_down(MouseButton::Right, {
            let (deck, window_id) = (f.deck.clone(), f.window);
            move |e, window, cx| {
                let d = deck.read(cx);
                let items = if one { tab_menu(&deck, pane, 0, window_id, d.layout(), cx) } else { pane_menu(&deck, pane, d.layout(), cx) };
                open_menu(e.position, items, window, cx);
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
            .child(icon(Icon::Grip, t.dim, t)),
    );

    // Tabs (or the title) keep priority over the pane buttons when the pane is narrow.
    // A header's title gives way after the pane buttons (they shrink far faster), never ×.
    if header {
        strip = strip.child(match titles.first() {
            Some(title) => tab_handlers(div().id(eid("tab", pane.0, 0)), f, pane, 0, title)
                .debug_selector(move || format!("tab-{}-0", pane.0))
                .flex()
                .flex_auto()
                .min_w_0()
                .h_full()
                .px_2()
                .items_center()
                .relative()
                .text_color(t.fg)
                .cursor_pointer()
                .tooltip(tip(title.clone()))
                .child(views::tab_label_element(&labels[0], t.fg, t))
                .when_some(flash, |d, n| d.child(flash_overlay("flash-tab", n, f.still, t)))
                .into_any_element(),
            None => div().flex_1().into_any_element(),
        });
    } else {
        let mut tabs = div()
            .flex()
            .flex_1()
            .min_w(t.tabs_min_w)
            .overflow_hidden()
            .debug_selector(move || format!("pane-tabs-{}", pane.0));
        for (i, title) in titles.iter().enumerate() {
            let active = i == p.active;
            let d_close = f.deck.clone();
            let full = title.clone();
            let (hover, line, fg, accent) = (t.hover, t.line, t.fg, t.accent);
            // Dragged over, a tab is a place: its left half before it, its right half after.
            let (moving, showing, dropping) = (f.tab_insert.clone(), f.tab_insert.clone(), f.tab_insert.clone());
            let d_drop = f.deck.clone();
            tabs = tabs.child(
                tab_handlers(div().id(eid("tab", pane.0, i)), f, pane, i, title)
                    .debug_selector(move || format!("tab-{}-{i}", pane.0))
                    .relative()
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
                    .hover(move |s| s.bg(hover))
                    .cursor_pointer()
                    .tooltip(tip(full))
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
                    // Drawn only (GPUI lays an element out without its drag-over style), so
                    // nothing moves (R9.10).
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
                    .child(views::tab_label_element(&labels[i], if active { t.fg } else { t.mute }, t))
                    .child(
                        div()
                            .id(eid("tab-x", pane.0, i))
                            .debug_selector(move || format!("tab-x-{}-{i}", pane.0))
                            .flex_none()
                            .px_1()
                            .rounded(t.radius_s)
                            .group("tab-close")
                            .hover(move |s| s.bg(line))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                apply_on(&d_close, cx, |l| l.close_tab(pane, i).map(|_| ()));
                            })
                            .child(icon(Icon::Close, t.dim, t).size(t.size(12.)).group_hover("tab-close", move |s| s.text_color(fg))),
                    )
                    // Focus: a line drawn over its top, not a border, so its label doesn't move
                    // (R9.10).
                    .when(active && focused, |d| {
                        d.child(div().debug_selector(move || format!("tab-focus-{}", pane.0)).absolute().top_0().left_0().w_full().h(t.hairline * 2.).bg(t.accent))
                    })
                    .when_some(flash.filter(|_| active), |d, n| d.child(flash_overlay("flash-tab", n, f.still, t))),
            );
        }
        strip = strip.child(tabs);
    }

    // Pane buttons.
    let is_detached = f.window != MAIN_WINDOW;
    let (d_col, d_max, d_det) = (f.deck.clone(), f.deck.clone(), f.deck.clone());
    let active = p.active;
    let can_detach = !p.tabs.is_empty() && !(is_detached && p.tabs.len() == 1);
    strip = strip.child(
        div()
            .debug_selector(move || format!("pane-buttons-{}", pane.0))
            .flex_shrink(if header { 100. } else { 1. })
            .min_w_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap_1()
            .px_1()
            // What the view offers now (R12.4), before the pane's own buttons.
            .children(f.actions.get(&pane).into_iter().flatten().enumerate().filter_map(|(i, a)| {
                let spec = crate::registry::by_id(a.id)?;
                let (deck, id) = (f.deck.clone(), a.id);
                let button = icon_button(eid("view-action", pane.0, i), a.icon, spec.label, t)
                    .debug_selector(move || format!("view-action-{}-{id}", pane.0))
                    .when(a.on, |b| b.bg(t.selected))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        run_view_action(&deck, pane, None, id, window, cx);
                    });
                Some(button)
            }))
            .when(!is_detached, |d| {
                d.child(
                    icon_button(("collapse", pane.0), if p.collapsed { Icon::Open } else { Icon::Collapse }, "collapse or expand", t)
                        .on_click(move |_, _, cx| apply_on(&d_col, cx, |l| l.toggle_collapse(pane))),
                )
                .child(
                    icon_button(("maximize", pane.0), if maximized { Icon::Restore } else { Icon::Maximize }, "maximize or restore", t)
                        .on_click(move |_, _, cx| apply_on(&d_max, cx, |l| l.toggle_maximize(pane))),
                )
            })
            .when(!p.tabs.is_empty(), |d| {
                let deck = f.deck.clone();
                d.child(
                    icon_button(("send", pane.0), Icon::Send, "send this pane's tabs to another pane", t).on_click(move |_, window, cx| {
                        crate::send_to::open(deck.clone(), SendSource::Pane(pane), window, cx);
                    }),
                )
            })
            .when(can_detach, |d| {
                d.child(
                    icon_button(("detach", pane.0), Icon::Detach, "detach tab to its own window", t)
                        .on_click(move |_, _, cx| apply_on(&d_det, cx, |l| l.detach(pane, active).map(|_| ()))),
                )
            }),
    );
    // A header's ×, in its own box: a narrow pane clips the buttons, never it.
    if one {
        let d_close = f.deck.clone();
        strip = strip.child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_1()
                .pr_1()
                .child(div().w(t.hairline).h(t.icon).bg(t.line))
                .child(
                    icon_button(("close", pane.0), Icon::Close, "close", t)
                        .debug_selector(move || format!("pane-close-{}", pane.0))
                        .on_click(move |_, _, cx| apply_on(&d_close, cx, |l| l.close_tab(pane, 0).map(|_| ()))),
                ),
        );
    }
    // A header's focus: a line drawn over its top, not a border, so nothing moves.
    if header && focused {
        strip = strip.child(
            div()
                .debug_selector(move || format!("pane-focus-{}", pane.0))
                .absolute()
                .top_0()
                .left_0()
                .w_full()
                .h(t.hairline * 2.)
                .bg(t.accent),
        );
    }

    if p.collapsed {
        return div().size_full().child(strip).into_any_element();
    }

    // Body, with drop zones while something is dragged.
    let d_focus = f.deck.clone();
    let window = f.window;
    let body = div()
        .id(("body", pane.0))
        .debug_selector(move || format!("pane-body-{}", pane.0))
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
            // An empty pane welcomes: the app's icon and name, and how to fill it.
            None => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(t.mute)
                .child(img(APP_ICON).flex_none().size(t.size(144.)))
                .child(div().text_color(t.hi).child(format!("Welcome to {APP_NAME}")))
                .child(label("Empty pane. Drop a tab here.").debug_selector(|| "empty-message".into()))
                .into_any_element(),
        });
    div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .min_w_0()
        .min_h_0()
        .child(strip)
        .child(body)
        .when_some(flash, |d, n| d.child(flash_overlay("flash-pane", n, f.still, t)))
        .into_any_element()
}

/// A revealed view's flash ([`Deck::flash`]): the accent around its tab and around its
/// pane, fading out over [`FLASH`]. A line over them, never a fill over the tab's label,
/// and nothing moves (R9.10);
/// GPUI draws frames only while it fades, and the deck drops it when its timer ends
/// (R1.3). Its id is the flash's number: a new flash fades again from the start. With
/// reduced motion (`still`) it stays drawn as it is until the deck drops it: GPUI would
/// draw a one-shot animation at its end, here invisible.
fn flash_overlay(name: &'static str, n: u64, still: bool, t: &Theme) -> AnyElement {
    let base = div().debug_selector(move || format!("{name}-{n}")).absolute().top_0().left_0().size_full().border_2().border_color(t.accent);
    if still {
        return base.into_any_element();
    }
    base.with_animation((name, n), Animation::new(FLASH), |d, delta| d.opacity(1. - delta)).into_any_element()
}


/// Under the panes while something is dragged (a tab, a pane, a list row, files from
/// outside): shows where it lands (R3.4, R3.13) and, on release, drops it at the pointer.
fn drop_layer(deck: &Entity<Deck>, window: WindowId, hint: &Rc<Cell<Option<(Drop, Rect)>>>, t: &Theme) -> impl IntoElement {
    let (m, strip) = (metrics(t), f32::from(t.size(16.)));
    let deck_id = deck.entity_id();
    let bounds: Rc<Cell<Bounds<Pixels>>> = Rc::default();
    // The drop under `at` in the layer drawn at `b`.
    let drop_at = move |l: &Layout, b: Bounds<Pixels>, at: Point<Pixels>| {
        if !b.contains(&at) {
            return None;
        }
        let size = (f32::from(b.size.width), f32::from(b.size.height));
        let local = (f32::from(at.x - b.origin.x), f32::from(at.y - b.origin.y));
        l.drop_at(window, size, m, strip, local).ok()?
    };
    // While dragging: the drop and its landing rectangle (none out of the layer, or for
    // another deck's drag), redrawn only when they change.
    let track = move |deck: &Entity<Deck>, hint: &Rc<Cell<Option<(Drop, Rect)>>>, b: Bounds<Pixels>, at: Point<Pixels>, moved: Option<Moved>, w: &mut Window, cx: &App| {
        let l = deck.read(cx).layout();
        let drop = moved.as_ref().and_then(|_| drop_at(l, b, at));
        if drop.is_some() && hint.get().map(|(d, _)| d) == drop {
            return;
        }
        // The preview does the drop on a copy: only when the drop changes.
        let next = moved.zip(drop).and_then(|(moved, drop)| Some((drop, l.preview(&moved, drop, (f32::from(b.size.width), f32::from(b.size.height)), m)?)));
        if hint.get() != next {
            hint.set(next);
            w.refresh();
        }
    };
    // On release: the drop at the pointer.
    let release = move |deck: &Entity<Deck>, bounds: &Rc<Cell<Bounds<Pixels>>>, w: &Window, cx: &App| drop_at(deck.read(cx).layout(), bounds.get(), w.mouse_position());
    let d = |_: u8| deck.clone();
    let h = |_: u8| hint.clone();
    let b = |_: u8| bounds.clone();
    let (d1, d2, d3, d4, a1, a2, a3, a4) = (d(1), d(2), d(3), d(4), d(5), d(6), d(7), d(8));
    let (h1, h2, h3, h4) = (h(1), h(2), h(3), h(4));
    let (b1, b2, b3, b4, b5) = (b(1), b(2), b(3), b(4), b(5));
    div()
        .id("drop-layer")
        .absolute()
        .size_full()
        .child(canvas(move |r, _, _| b5.set(r), |_, _, _, _| {}).size_full())
        .on_drag_move::<DraggedTab>(move |e, w, cx| {
            let tab = e.drag(cx);
            let moved = (tab.deck == deck_id).then_some(Moved::Tab(tab.pane, tab.index));
            track(&d1, &h1, e.bounds, e.event.position, moved, w, cx);
        })
        .on_drag_move::<DraggedPane>(move |e, w, cx| {
            let pane = e.drag(cx);
            let moved = (pane.deck == deck_id).then_some(Moved::Pane(pane.pane));
            track(&d2, &h2, e.bounds, e.event.position, moved, w, cx);
        })
        .on_drag_move::<DraggedListItem>(move |e, w, cx| {
            let item = e.drag(cx);
            let moved = (item.deck == deck_id).then(|| Moved::View(item.view.clone()));
            track(&d3, &h3, e.bounds, e.event.position, moved, w, cx);
        })
        .on_drag_move::<ExternalPaths>(move |e, w, cx| {
            // Files land where their first one does (they share its pane); nothing is read
            // from the disk outside the panes, and nothing shows where they can't open.
            let inside = e.bounds.contains(&e.event.position) && d4.read(cx).has_services();
            let moved = inside.then(|| crate::drops::sort(e.drag(cx).paths()).0.into_iter().next().map(Moved::View)).flatten();
            track(&d4, &h4, e.bounds, e.event.position, moved, w, cx);
        })
        .on_drop(move |tab: &DraggedTab, w, cx| {
            if let Some(drop) = release(&a1, &b1, w, cx).filter(|_| tab.deck == deck_id) {
                apply_on(&a1, cx, |l| l.move_tab(tab.pane, tab.index, drop));
            }
        })
        .on_drop(move |pane: &DraggedPane, w, cx| {
            if let Some(drop) = release(&a2, &b2, w, cx).filter(|_| pane.deck == deck_id) {
                apply_on(&a2, cx, |l| l.move_pane(pane.pane, drop));
            }
        })
        .on_drop(move |item: &DraggedListItem, w, cx| {
            if let Some(drop) = release(&a3, &b3, w, cx).filter(|_| item.deck == deck_id) {
                let view = item.view.clone();
                apply_on(&a3, cx, |l| sidebar::open_at(l, view, drop));
            }
        })
        .on_drop(move |paths: &ExternalPaths, w, cx| {
            if let Some(drop) = release(&a4, &b4, w, cx) {
                crate::drops::open_files(&a4, paths.paths(), drop, cx);
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
