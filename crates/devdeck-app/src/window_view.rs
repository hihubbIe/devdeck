//! Root view of one OS window: title bar, pane tree, status bar, window edges.
//! The main window shows the active workspace's `Deck` and switches with it; a
//! detached window always belongs to the deck that opened it.
//!
//! Rendering works on a snapshot (`Frame`) taken from the deck at the start of
//! `render`, so drawing never holds a borrow of the deck.

use crate::actions::*;
use crate::deck::Deck;
use crate::stats;
use crate::app_settings;
use crate::settings_view::SettingsView;
use crate::shell::{self, Shell};
use crate::sidebar::{self, Activity, DraggedListItem, Sidebar, SidebarEscape};
use crate::views;
use devdeck_ui::{DragGhost, Empty, MenuItem, Theme, apply_window_scale, button, icon_button, label, logical, open_menu, theme, tip};
use devdeck_core::layout::{Axis, Direction, Drop, Edge, Layout, MAIN_WINDOW, Node, PaneId, SplitId, WindowId};
use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, Context, CursorStyle, Decorations, DragMoveEvent, Entity, EntityId,
    FocusHandle, Focusable, Hitbox, HitboxBehavior, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Point, Render, ResizeEdge, SharedString, Size, StatefulInteractiveElement, Styled, Subscription, WeakEntity,
    WeakFocusHandle, Window, WindowBounds, WindowDecorations, WindowOptions, canvas, div, point, prelude::*, relative,
    size,
};
use std::collections::HashMap;

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
    /// Where focus was when Settings opened.
    before_settings: Option<WeakFocusHandle>,
    deck_sub: Subscription,
    /// Main window: the active workspace's sidebar.
    sidebar: Option<(Entity<Sidebar>, Subscription)>,
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

fn logical_size(window: &Window) -> (f32, f32) {
    let s = window.bounds().size;
    (f32::from(s.width), f32::from(s.height))
}

/// The workspace menu (title-bar button): alphabetical, current one checked.
pub fn workspace_menu_items(shell: &Entity<Shell>, cx: &App) -> Vec<MenuItem> {
    let s = shell.read(cx);
    s.workspaces
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let item = MenuItem::new(w.name.clone(), move |_, cx| shell::switch(i, cx)).checked(i == s.active);
            if i < 9 { item.hint(format!("ctrl alt {}", i + 1)) } else { item }
        })
        .collect()
}

/// Everything a render pass needs, copied out of the deck.
struct Frame<'a> {
    layout: &'a Layout,
    views: &'a HashMap<PaneId, AnyView>,
    window: WindowId,
    deck: &'a Entity<Deck>,
    t: &'a Theme,
    dragging: bool,
}

impl WindowView {
    /// The main window: shows the shell's active workspace. Closing it asks to quit.
    pub fn main(shell: Entity<Shell>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let deck = shell.read(cx).active_deck();
        let mut observe = vec![
            cx.observe(&app_settings::settings(cx), |_, _, cx| cx.notify()),
            cx.observe_in(&shell, window, |this, shell, window, cx| this.follow_shell(&shell, window, cx)),
        ];
        let size_shell = shell.downgrade();
        observe.push(cx.observe_window_bounds(window, move |_, window, cx| {
            let size = logical_size(window);
            let _ = size_shell.update(cx, |s, _| s.main_size = Some(size));
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
            let size = logical_size(window);
            let _ = size_deck.update(cx, |d, cx| d.record_size(id, size, cx));
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
        observe: Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
            before_settings: None,
            deck_sub,
            sidebar: None,
            view_target: None,
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
        let target = self.focused_pane(cx).map(|p| (p, self.deck.read(cx).layout.pane(p).ok().and_then(|p| p.active_view().cloned())));
        if target == self.view_target {
            return;
        }
        self.view_target = target.clone();
        let Some((pane, _)) = target else { return };
        if !self.panes_focus.contains_focused(window, cx) {
            return;
        }
        match self.deck.read(cx).view_focus(pane) {
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
            self.deck = deck;
            self.view_target = None;
            let sidebar = shell.read(cx).active_sidebar();
            self.sidebar = Some((sidebar.clone(), cx.observe(&sidebar, |_, _, cx| cx.notify())));
            window.set_window_title(&format!("devdeck · {}", shell.read(cx).active_name()));
            if !app_settings::is_open(cx) {
                match self.remembered.get(&self.deck.entity_id()).and_then(WeakFocusHandle::upgrade) {
                    Some(h) => h.focus(window, cx),
                    None => self.panes_focus.focus(window, cx),
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
                None => self.panes_focus.focus(window, cx),
            }
        }
        cx.notify();
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
            self.panes_focus.focus(window, cx);
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
        self.deck.read(cx).layout.window(self.id).ok().map(|w| w.focused)
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
        (f32::from(w), f32::from(s.height - t.title_h - t.status_h))
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
        let is_main = self.is_main();
        let settings_open = is_main && app_settings::is_open(cx);
        if settings_open && self.settings.is_none() {
            self.settings = Some(cx.new(SettingsView::new));
        }
        let (layout, views, live, last_error) = {
            let d = self.deck.read(cx);
            let views: HashMap<PaneId, AnyView> = d
                .layout
                .panes
                .iter()
                .filter_map(|(id, p)| p.active_view().and_then(|r| d.view(r)).map(|v| (*id, v)))
                .collect();
            (d.layout.clone(), views, d.live_views(), d.last_error.clone())
        };
        let Ok(w) = layout.window(self.id) else {
            return div().size_full().bg(t.bg).into_any_element();
        };
        let frame = Frame { layout: &layout, views: &views, window: self.id, deck: &self.deck, t: &t, dragging: cx.has_active_drag() };
        let deck_id = self.deck.entity_id();
        let workspace = self.deck.read(cx).workspace.clone();
        let shell = self.shell.as_ref().and_then(WeakEntity::upgrade);
        let ws_name = shell.as_ref().map_or(workspace, |s| s.read(cx).active_name().to_string());
        let tree = match w.maximized {
            Some(p) => render_pane(&frame, p, None),
            None => render_node(&frame, &w.root, None),
        };
        let status = format!(
            "{ws_name} · {} · focused pane {} · {} views · {} renders{}",
            if is_main { "main window" } else { "detached window" },
            w.focused.0,
            live,
            stats::renders(),
            last_error.map(|e| format!(" · {e}")).unwrap_or_default()
        );
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
                        .tooltip(tip("switch workspace (ctrl alt 1…9)"))
                        .on_click(move |e, window, cx| {
                            let items = workspace_menu_items(&shell, cx);
                            open_menu(e.position(), items, window, cx);
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
                        .tooltip(tip("env for runs and the cluster list (ctrl e)"))
                        .on_click(move |e, window, cx| {
                            if let Some(s) = &sidebar {
                                open_menu(e.position(), Sidebar::env_menu(s, cx), window, cx);
                            }
                        }),
                )
            })
            .when(!is_main, |d| d.child(div().text_color(t.mute).child(format!("{ws_name} · detached window"))))
            .child(div().flex_1())
            .when(is_main, |d| {
                d.child(
                    button("settings", if settings_open { "⚙ settings ✓" } else { "⚙ settings" }, &t)
                        .tooltip(tip("settings (ctrl ,)"))
                        .on_click(cx.listener(move |this, _, window, cx| this.set_settings_open(!settings_open, window, cx))),
                )
            })
            .when(!is_main, |d| {
                d.child(
                    button("reattach", "⇲", &t)
                        .tooltip(tip("send back to the main window (ctrl shift a)"))
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

        let status_bar = div()
            .flex_none()
            .h(t.status_h)
            .px_3()
            .flex()
            .items_center()
            .bg(t.bar)
            .border_t_1()
            .border_color(t.line)
            .text_color(t.mute)
            .child(div().flex_1().min_w_0().truncate().child(status));

        let panes = div()
            .id("panes")
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
            .on_action(cx.listener(|this, _: &GrowPane, _, cx| this.with_focused(cx, |l, p| l.grow(p, 0.05))))
            .on_action(cx.listener(|this, _: &ShrinkPane, _, cx| this.with_focused(cx, |l, p| l.grow(p, -0.05))))
            .on_action(cx.listener(|this, _: &ToggleFollow, _, cx| {
                let Some(p) = this.focused_pane(cx) else { return };
                let deck = this.deck.read(cx);
                let view = deck.layout.pane(p).ok().and_then(|p| p.active_view().cloned()).and_then(|r| deck.view(&r));
                if let Some(v) = view {
                    views::toggle_follow(&v, cx);
                }
            }))
            // Per-workspace id, so element state never carries over between workspaces.
            .child(div().id(("ws", deck_id.as_u64())).size_full().min_w_0().min_h_0().flex().child(tree));
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
                        .when(visible, |d| d.child(sidebar.clone()))
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
            .on_action(cx.listener(|this, _: &ToggleSettings, w, cx| {
                let open = app_settings::is_open(cx);
                this.set_settings_open(!open, w, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseSettings, w, cx| this.set_settings_open(false, w, cx)))
            .on_action(|_: &IncreaseScale, _, cx| app_settings::increase(cx))
            .on_action(|_: &DecreaseScale, _, cx| app_settings::decrease(cx))
            .on_action(|_: &ResetScale, _, cx| app_settings::reset(cx))
            .on_action(cx.listener(|this, _: &Reattach, _, cx| this.reattach(cx)))
            .on_action(cx.listener(|this, _: &ShowApps, w, cx| this.show_activity(Activity::Apps, w, cx)))
            .on_action(cx.listener(|this, _: &ShowCluster, w, cx| this.show_activity(Activity::Cluster, w, cx)))
            .on_action(cx.listener(|this, _: &ShowPorts, w, cx| this.show_activity(Activity::Ports, w, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, w, cx| {
                let visible = this.active_sidebar().is_some_and(|s| s.read(cx).state.visible);
                this.set_sidebar_visible(!visible, w, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenEnvMenu, w, cx| this.open_env_menu(w, cx)))
            // Esc with an empty filter: back to the panes.
            .on_action(cx.listener(|this, _: &SidebarEscape, w, cx| this.panes_focus.focus(w, cx)))
            .child(title_bar)
            .child(body)
            .child(status_bar)
            .children(devdeck_ui::render_layer(window, cx))
            .when(is_main, |d| d.child(devdeck_ui::toasts(cx)));

        match window.window_decorations() {
            Decorations::Server => root.into_any_element(),
            Decorations::Client { .. } => {
                let edge = t.window_edge;
                div()
                .id("csd")
                .size_full()
                .relative()
                .capture_any_mouse_down(move |e, window, cx| {
                    let size = window.window_bounds().get_bounds().size;
                    if let Some(edge) = resize_edge(e.position, size, edge) {
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
                .tooltip(tip(format!("{} ({})", a.title(), a.key_hint())))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if on {
                        this.set_sidebar_visible(false, window, cx);
                    } else {
                        this.show_activity(a, window, cx);
                    }
                }))
                .child(a.glyph()),
        );
    }
    bar
}

/// Sets resize cursors along the window border (client-side decorations; GNOME on
/// Wayland draws none). The resize itself starts in `capture_any_mouse_down`.
fn window_edge_cursors(edge_width: Pixels) -> impl IntoElement {
    canvas(
        |_bounds, window, _cx| {
            let size = window.window_bounds().get_bounds().size;
            window.insert_hitbox(gpui::Bounds::new(point(Pixels::ZERO, Pixels::ZERO), size), HitboxBehavior::Normal)
        },
        move |_bounds, hitbox: Hitbox, window, _cx| {
            let size = window.window_bounds().get_bounds().size;
            if let Some(edge) = resize_edge(window.mouse_position(), size, edge_width) {
                let style = match edge {
                    ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                    ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
                    ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
                    ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
                };
                window.set_cursor_style(style, &hitbox);
            }
        },
    )
    .absolute()
    .size_full()
}

fn resize_edge(pos: Point<Pixels>, size: Size<Pixels>, edge: Pixels) -> Option<ResizeEdge> {
    let (l, r, t, b) = (pos.x < edge, pos.x > size.width - edge, pos.y < edge, pos.y > size.height - edge);
    Some(match (t, b, l, r) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, ..) => ResizeEdge::Top,
        (_, true, ..) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
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
        .on_drop(move |tab: &DraggedTab, _, cx| {
            if tab.deck == deck_id {
                let tab = tab.clone();
                apply_on(&strip_deck, cx, |l| l.move_tab(tab.pane, tab.index, Drop::Into(pane)));
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
        let (hover, line, fg) = (t.hover, t.line, t.fg);
        tabs = tabs.child(
            div()
                .id(eid("tab", pane.0, i))
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
                .on_drag(DraggedTab { deck: deck_id, pane, index: i, title: title.clone() }, |d, _, _, cx| {
                    cx.new(|_| DragGhost(d.title.clone()))
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
                    icon_button(("collapse", pane.0), if p.collapsed { "▾" } else { "▴" }, "collapse or expand (ctrl shift c)", t)
                        .on_click(move |_, _, cx| apply_on(&d_col, cx, |l| l.toggle_collapse(pane))),
                )
                .child(
                    icon_button(("maximize", pane.0), if maximized { "◱" } else { "□" }, "maximize or restore (ctrl shift m)", t)
                        .on_click(move |_, _, cx| apply_on(&d_max, cx, |l| l.toggle_maximize(pane))),
                )
            })
            .when(can_detach, |d| {
                d.child(
                    icon_button(("detach", pane.0), "⇱", "detach tab to its own window (ctrl shift d)", t)
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
            let needs = d_focus.read(cx).layout.window(window).is_ok_and(|w| w.focused != pane);
            if needs {
                apply_on(&d_focus, cx, |l| l.focus(pane));
            }
        })
        .child(match f.views.get(&pane) {
            Some(v) => div().size_full().child(v.clone()).into_any_element(),
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
