//! The notification center (plan "notifications", step A; `docs/DESIGN.md`
//! "Notifications"): every toast is kept in `devdeck-ui`'s store, shown here in a
//! full-window view covering the main window's panes, as Settings does (one cover at a
//! time). The title bar's bell, in every window, says what is unread.
//!
//! - Notifications are read when the center shows them, when their toast is closed, or
//!   when one of their actions is used. Rows unread when shown stay lighter while the
//!   center is open.
//! - A notification's actions are data ([`NoteAction`]): the app reads their payload at
//!   the click ([`resolve`]). A toast's closures stay on the toast.
//! - A notification's source is a workspace's id (`None`: the app's), named here.

use crate::shell;
use devdeck_core::layout::ViewRef;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::overlay::{Note, kind_look};
use devdeck_ui::toolbar::{MenuValue, ToolAction, ToolItem, Toolbar, ToolbarState};
use devdeck_ui::{Icon, MenuItem, NoteAction, Theme, ToastKind, button, icon, label, theme, tip, toasts};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ListAlignment, ListState, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, actions, div, list, prelude::*,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

actions!(notifications, [ToggleNotifications, CloseNotifications, NextNote, PrevNote, NextAction, PrevAction, RunNote, DeleteNote, ToggleUnread, WorkspaceMenu, MarkAllRead, ClearNotes]);

// ------------------------------------------------------------------ actions as data

/// Separates a payload's fields (never in an id or a view address).
const SEP: char = '\u{1f}';

/// Opens `view` in `workspace` (shows the workspace first).
pub fn open_action(label: impl Into<SharedString>, workspace: &str, view: &ViewRef) -> NoteAction {
    NoteAction::new(label, format!("open{SEP}{workspace}{SEP}{}", view.0))
}

/// Opens the problems view of the workspace on screen.
pub fn problems_action(label: impl Into<SharedString>) -> NoteAction {
    NoteAction::new(label, "problems")
}

/// What a payload says.
#[derive(Debug, PartialEq, Eq)]
enum Act {
    Open { workspace: String, view: ViewRef },
    Problems,
}

fn decode(payload: &str) -> Option<Act> {
    let mut parts = payload.split(SEP);
    match parts.next()? {
        "open" => {
            let workspace = parts.next()?.to_string();
            let view = ViewRef(parts.next()?.to_string());
            Some(Act::Open { workspace, view })
        }
        "problems" => Some(Act::Problems),
        _ => None,
    }
}

/// Does what `action` says: the toasts' and the center's resolver. What it opens takes
/// the keys, and the center makes way for it. A workspace gone says so in a toast and
/// leaves the center as it is; a payload not understood does nothing (logged).
pub fn resolve(action: &NoteAction, _window: &mut Window, cx: &mut App) {
    let Some(s) = shell::try_shell(cx) else { return };
    let (switch_to, deck, addr) = match decode(&action.payload) {
        Some(Act::Open { workspace, view }) => match s.read(cx).workspaces.iter().enumerate().find(|(_, w)| w.id == workspace).map(|(i, w)| (i, w.deck.downgrade())) {
            Some((i, deck)) => (Some(i), deck, ViewAddr::parse(&view)),
            None => {
                let text = format!("{}: the workspace {workspace} is gone", action.label);
                toasts(cx).update(cx, |t, cx| t.push(ToastKind::Warn, text, vec![], cx));
                return;
            }
        },
        Some(Act::Problems) => (None, s.read(cx).active_deck().downgrade(), ViewAddr::Problems),
        None => {
            log::warn!("notification action not understood: {}", action.payload);
            return;
        }
    };
    if let Some(i) = switch_to {
        shell::switch(i, cx);
    }
    crate::views::open_in(&deck, addr, cx);
    // Deferred: this may run inside the main window's update.
    if let Some(main) = s.read(cx).main_window() {
        cx.defer(move |cx| {
            let _ = main.update(cx, |w, window, cx| w.uncover_to_panes(window, cx));
        });
    }
}

/// Opens the center in the main window (brought forward), on notification `on` if
/// given. Deferred: it may come from inside a window's update (a toast's click).
pub fn open_center(on: Option<u64>, cx: &mut App) {
    cx.defer(move |cx| {
        let Some(main) = shell::try_shell(cx).and_then(|s| s.read(cx).main_window()) else { return };
        let _ = main.update(cx, |w, window, cx| {
            window.activate_window();
            w.open_notifications(on, window, cx);
        });
    });
}

/// The toasts' hooks: their "+N more" and text clicks open the center, their actions as
/// data are resolved here.
pub fn install(cx: &mut App) {
    toasts(cx).update(cx, |t, _| t.set_hooks(Rc::new(|on, _, cx| open_center(on, cx)), Rc::new(resolve)));
}

// ------------------------------------------------------------------ the bell

/// What the bell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    /// Nothing unread: the bell alone.
    Clear,
    /// Unread, no error: a small blue dot.
    Unread(usize),
    /// Unread errors: a larger red dot with their count (red wins over blue).
    Errors { errors: usize, unread: usize },
}

pub fn badge(cx: &mut App) -> Badge {
    let t = toasts(cx);
    let s = &t.read(cx).notes;
    match (s.unread(), s.unread_errors()) {
        (0, _) => Badge::Clear,
        (unread, 0) => Badge::Unread(unread),
        (unread, errors) => Badge::Errors { errors, unread },
    }
}

/// The bell's tooltip: its name and the counts in words (never a key, R5.4).
pub fn badge_words(b: Badge) -> String {
    let unread = |n: usize| format!("{n} unread");
    match b {
        Badge::Clear => "notifications".into(),
        Badge::Unread(n) => format!("notifications: {}", unread(n)),
        Badge::Errors { errors, unread: n } => format!("notifications: {}, {errors} {}", unread(n), if errors == 1 { "error" } else { "errors" }),
    }
}

/// The bell button (title bar), with its badge. The caller adds the click.
pub fn bell(b: Badge, t: &Theme) -> gpui::Stateful<gpui::Div> {
    let mark = match b {
        Badge::Clear => None,
        Badge::Unread(_) => Some(
            div().debug_selector(|| "bell-dot".into()).absolute().top(t.size(4.)).right(t.size(4.)).size(t.size(7.)).rounded_full().bg(t.accent).into_any_element(),
        ),
        Badge::Errors { errors, .. } => Some(
            div()
                .debug_selector(|| "bell-count".into())
                .absolute()
                .top(t.size(0.))
                .right(t.size(-2.))
                .h(t.size(13.))
                .min_w(t.size(13.))
                .px(t.size(3.))
                .rounded_full()
                .bg(t.err)
                .flex()
                .items_center()
                .justify_center()
                .text_size(t.size(9.))
                .line_height(t.size(13.))
                .text_color(t.hi)
                .child(if errors > 99 { "99+".to_string() } else { errors.to_string() })
                .into_any_element(),
        ),
    };
    let hover = t.hover;
    div()
        .id("bell")
        .debug_selector(|| "bell".into())
        .relative()
        .w(t.icon_button)
        .h(t.icon_button)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(t.radius_s)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .tooltip(tip(badge_words(b)))
        .child(devdeck_ui::button_icon(Icon::Bell, t))
        .children(mark)
}

// ------------------------------------------------------------------ the center

/// Which notifications show, by source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    All,
    /// The app's own (no workspace).
    App,
    Workspace(String),
}

impl Source {
    fn takes(&self, n: &Note) -> bool {
        match self {
            Source::All => true,
            Source::App => n.source.is_none(),
            Source::Workspace(id) => n.source.as_deref() == Some(id.as_str()),
        }
    }
}

/// "2 min ago".
pub fn ago(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86400),
    }
}

/// A workspace's name from its id (the id when it's gone); the app's: "app".
fn source_name(source: Option<&str>, cx: &App) -> SharedString {
    let Some(id) = source else { return "app".into() };
    shell::try_shell(cx).and_then(|s| s.read(cx).workspaces.iter().find(|w| w.id == id).map(|w| w.name.clone().into())).unwrap_or_else(|| id.to_string().into())
}

pub struct NotificationsView {
    focus: FocusHandle,
    list: ListState,
    bar: Rc<ToolbarState>,
    /// Only what was unread (or shown unread during this visit).
    pub unread_only: bool,
    pub source: Source,
    /// The selected notification.
    pub selected: Option<u64>,
    /// The selected notification's action Enter runs (← → move it).
    pub action: usize,
    /// Shown unread during this visit: drawn lighter, kept by "unread".
    fresh: Rc<RefCell<HashSet<u64>>>,
    /// The notifications shown, in order (ids).
    rows: Vec<u64>,
    _observe: Subscription,
}

impl Focusable for NotificationsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl NotificationsView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = toasts(cx);
        let observe = cx.observe(&store, |this, _, cx| {
            this.sync(cx);
            cx.notify();
        });
        let mut this = Self {
            focus: cx.focus_handle(),
            // Rows measured beyond the visible ones, so scrolling doesn't pop them in
            // (only those in view are read).
            list: ListState::new(0, ListAlignment::Top, theme(cx).size(400.)),
            bar: ToolbarState::new(),
            unread_only: false,
            source: Source::All,
            selected: None,
            action: 0,
            fresh: Rc::default(),
            rows: Vec::new(),
            _observe: observe,
        };
        this.rebuild(cx);
        this
    }

    /// Selects notification `id`, its first action.
    pub fn select(&mut self, id: Option<u64>) {
        if self.selected != id {
            self.selected = id;
            self.action = 0;
        }
    }

    /// The id of the row at the top of the view.
    pub fn top_row(&self) -> Option<u64> {
        self.rows.get(self.list.logical_scroll_top().item_ix).copied()
    }

    /// The ids shown now, newest first.
    pub fn rows(&self) -> &[u64] {
        &self.rows
    }

    fn shows(&self, n: &Note) -> bool {
        self.source.takes(n) && (!self.unread_only || !n.read || self.fresh.borrow().contains(&n.id))
    }

    fn wanted(&self, cx: &mut Context<Self>) -> Vec<u64> {
        let store = toasts(cx);
        store.read(cx).notes.notes().iter().filter(|n| self.shows(n)).map(|n| n.id).collect()
    }

    /// Rows from the store: the list takes only what changed (a notification come, one
    /// removed), so its scroll stays where it was.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let rows = self.wanted(cx);
        if rows != self.rows {
            let (old, new) = changed_span(&self.rows, &rows);
            self.list.splice(old, new);
            self.rows = rows;
        }
        self.keep_selection();
    }

    /// Rows from the store and new filters: the list starts again.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.rows = self.wanted(cx);
        self.list.reset(self.rows.len());
        self.keep_selection();
    }

    fn keep_selection(&mut self) {
        if self.selected.is_some_and(|id| !self.rows.contains(&id)) {
            self.select(self.rows.first().copied());
        }
    }

    /// A new visit: on notification `on` (shown whatever the filters say), else the
    /// newest.
    pub fn show(&mut self, on: Option<u64>, cx: &mut Context<Self>) {
        self.fresh.borrow_mut().clear();
        if let Some(id) = on {
            let store = toasts(cx);
            if let Some(n) = store.read(cx).notes.get(id).cloned()
                && !self.source.takes(&n)
            {
                self.source = Source::All;
            }
            self.unread_only = false;
        }
        self.rebuild(cx);
        self.select(on.filter(|id| self.rows.contains(id)).or_else(|| self.rows.first().copied()));
        self.reveal();
        cx.notify();
    }

    fn reveal(&self) {
        if let Some(i) = self.selected.and_then(|id| self.rows.iter().position(|r| *r == id)) {
            self.list.scroll_to_reveal_item(i);
        }
    }

    fn step(&mut self, dir: isize, cx: &mut Context<Self>) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        let at = self.selected.and_then(|id| self.rows.iter().position(|r| *r == id)).map_or(if dir > 0 { -1 } else { n }, |i| i as isize);
        let next = (at + dir).clamp(0, n - 1) as usize;
        self.select(Some(self.rows[next]));
        self.reveal();
        cx.notify();
    }

    /// Moves the action Enter runs among the selected notification's.
    fn step_action(&mut self, dir: isize, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let n = toasts(cx).read(cx).notes.get(id).map_or(0, |n| n.actions.len());
        if n > 0 {
            self.action = (self.action as isize + dir).clamp(0, n as isize - 1) as usize;
            cx.notify();
        }
    }

    pub fn set_unread_only(&mut self, on: bool, cx: &mut Context<Self>) {
        self.unread_only = on;
        self.rebuild(cx);
        cx.notify();
    }

    pub fn set_source(&mut self, source: Source, cx: &mut Context<Self>) {
        self.source = source;
        self.rebuild(cx);
        cx.notify();
    }

    /// Marks read what the filters show.
    pub fn mark_all_read(&mut self, cx: &mut Context<Self>) {
        let source = self.source.clone();
        toasts(cx).update(cx, |t, cx| {
            if t.notes.mark_read_where(|n| source.takes(n)) > 0 {
                cx.notify();
            }
        });
    }

    /// Removes what the filters show (the store only: the control API's history stays).
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let rows: HashSet<u64> = self.rows.iter().copied().collect();
        toasts(cx).update(cx, |t, cx| {
            t.notes.delete_where(|n| rows.contains(&n.id));
            cx.notify();
        });
    }

    /// Removes the selected notification; the next one is selected.
    pub fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let at = self.rows.iter().position(|r| *r == id).unwrap_or(0);
        let next = self.rows.get(at + 1).or_else(|| at.checked_sub(1).and_then(|i| self.rows.get(i))).copied();
        self.select(next);
        toasts(cx).update(cx, |t, cx| {
            t.notes.delete(id);
            cx.notify();
        });
        self.sync(cx);
        self.reveal();
        cx.notify();
    }

    /// Runs the selected notification's selected action (its first unless ← → moved).
    pub fn run_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let action = toasts(cx).read(cx).notes.get(id).and_then(|n| n.actions.get(self.action).or(n.actions.first()).cloned());
        if let Some(a) = action {
            run_action(id, a, window, cx);
        }
    }

    fn workspace_menu(this: &Entity<Self>, cx: &App) -> Vec<MenuItem> {
        let current = this.read(cx).source.clone();
        let pick = |source: Source| {
            let this = this.clone();
            move |_: &mut Window, cx: &mut App| this.update(cx, |v, cx| v.set_source(source.clone(), cx))
        };
        let mut items = vec![
            MenuItem::new("all", pick(Source::All)).checked(current == Source::All),
            MenuItem::new("app", pick(Source::App)).checked(current == Source::App),
        ];
        if let Some(s) = shell::try_shell(cx) {
            let mut named: Vec<(String, String)> = s.read(cx).workspaces.iter().map(|w| (w.name.clone(), w.id.clone())).collect();
            named.sort_by_key(|(n, _)| n.to_lowercase());
            for (name, id) in named {
                let on = current == Source::Workspace(id.clone());
                items.push(MenuItem::new(name, pick(Source::Workspace(id))).checked(on));
            }
        }
        items
    }

    pub fn toolbar(&self, cx: &mut Context<Self>) -> Toolbar {
        let me = cx.entity();
        let run = |f: fn(&mut Self, &mut Context<Self>)| {
            let me = me.clone();
            move |_: &mut Window, cx: &mut App| me.update(cx, f)
        };
        let store = toasts(cx);
        let s = &store.read(cx).notes;
        let shown_unread = s.notes().iter().any(|n| !n.read && self.source.takes(n));
        let value = match &self.source {
            Source::All => "all".into(),
            Source::App => "app".into(),
            Source::Workspace(id) => source_name(Some(id), cx),
        };
        let me_menu = me.clone();
        Toolbar::new("notes-toolbar", self.bar.clone())
            .item(ToolItem::Action(
                ToolAction::new("notes.unread", "unread", run(|v, cx| v.set_unread_only(!v.unread_only, cx))).icon(Icon::Unread).toggle(self.unread_only).state_shown(self.unread_only),
            ))
            .item(ToolItem::Menu {
                action: ToolAction::new("notes.workspace", "workspace", |_, _| {}).icon(Icon::Filter).state_shown(self.source != Source::All),
                value: Some(MenuValue::new(value)),
                max_chars: 16,
                items: Rc::new(move |cx| Self::workspace_menu(&me_menu, cx)),
            })
            .item(ToolItem::Spacer)
            .item(ToolItem::Action(
                ToolAction::new("notes.read_all", "mark all read", run(Self::mark_all_read)).icon(Icon::MarkRead).why_not((!shown_unread).then(|| "nothing unread".into())),
            ))
            .item(ToolItem::Action(ToolAction::new("notes.clear", "clear", run(Self::clear)).icon(Icon::Clear).why_not(self.rows.is_empty().then(|| "nothing to clear".into()))))
    }
}

/// What changed from `old` to `new`, as a list splice: the range of `old` replaced and
/// how many rows take its place (the common start and end kept).
pub fn changed_span(old: &[u64], new: &[u64]) -> (std::ops::Range<usize>, usize) {
    let start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - start;
    let end = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
    (start..old.len() - end, new.len() - start - end)
}

/// Runs a notification's action: it is read, then the app does what it says.
fn run_action(id: u64, action: NoteAction, window: &mut Window, cx: &mut App) {
    let store = toasts(cx);
    store.update(cx, |t, cx| t.mark_read(id, cx));
    if let Some(resolve) = store.read(cx).resolver() {
        resolve(&action, window, cx);
    }
}

/// One row's data, copied out of the store for the list.
struct Row {
    note: Note,
    source: SharedString,
}

/// One row; `selected`: the action Enter runs, when the row is the selected one.
fn render_row(row: &Row, selected: Option<usize>, fresh: bool, view: WeakEntity<NotificationsView>, now: std::time::Instant, t: &Theme) -> gpui::AnyElement {
    let n = &row.note;
    let id = n.id;
    let (kind, color, _) = kind_look(n.kind, t);
    let mut buttons = div().flex().flex_wrap().gap_1();
    for (j, a) in n.actions.iter().enumerate() {
        let a = a.clone();
        buttons = buttons.child(
            button(("note-action", (id as usize) * 16 + j), a.label.clone(), t)
                .debug_selector(move || format!("note-action-{id}-{j}"))
                .border_1()
                .border_color(if selected == Some(j) { t.accent } else { t.line })
                .on_click(move |_, window, cx| run_action(id, a.clone(), window, cx)),
        );
    }
    let select = view.clone();
    div()
        .id(("note-row", id as usize))
        .debug_selector(move || format!("note-{id}"))
        .w_full()
        .flex()
        .items_start()
        .gap_2()
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(t.line_soft)
        .when(fresh, |d| d.bg(t.raised))
        .when(selected.is_some(), |d| d.bg(t.selected))
        .on_click(move |_, _, cx| {
            let _ = select.update(cx, |v, cx| {
                v.select(Some(id));
                cx.notify();
            });
        })
        .child(div().flex_none().h(t.line_height).flex().items_center().child(icon(kind, color, t)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                // The title and the whole text, wrapped (the one place it is never cut,
                // R4.4's opt-in), beside where it came from and when.
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap_3()
                        .min_w_0()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .when_some(n.title.clone(), |d, title| d.child(label(title).text_color(if fresh { t.hi } else { t.fg })))
                                .child(div().whitespace_normal().text_color(if fresh { t.hi } else { t.fg }).child(n.text.clone())),
                        )
                        .child(label(row.source.clone()).flex_none().max_w(t.size(200.)).text_color(t.mute))
                        .child(div().flex_none().whitespace_nowrap().text_color(t.mute).child(ago(now.saturating_duration_since(n.at)))),
                )
                .when(!n.actions.is_empty(), |d| d.child(buttons)),
        )
        .into_any_element()
}

impl Render for NotificationsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        self.sync(cx);
        let toolbar = self.toolbar(cx);
        let store = toasts(cx);
        let rows: Rc<Vec<Row>> = Rc::new({
            let s = &store.read(cx).notes;
            self.rows.iter().filter_map(|id| s.get(*id).cloned()).map(|note| Row { source: source_name(note.source.as_deref(), cx), note }).collect()
        });
        let now = cx.background_executor().now();
        let body = if rows.is_empty() {
            let text = if store.read(cx).notes.is_empty() { "no notifications" } else { "nothing matches" };
            div().flex_1().flex().items_center().justify_center().text_color(t.mute).child(label(text)).into_any_element()
        } else {
            let (view, selected, action, fresh) = (cx.entity().downgrade(), self.selected, self.action, self.fresh.clone());
            let (theme, state) = (t.clone(), self.list.clone());
            list(self.list.clone(), move |i, _, cx| {
                let Some(row) = rows.get(i) else { return div().into_any_element() };
                let id = row.note.id;
                // In view (not only drawn ahead in the overdraw): read from now on, drawn
                // lighter during this visit. Known once laid out, so asked after.
                if !row.note.read {
                    let (state, fresh) = (state.clone(), fresh.clone());
                    cx.defer(move |cx| {
                        let in_view = state.item_is_above_viewport(i) == Some(false) && state.item_is_below_viewport(i) == Some(false);
                        if in_view {
                            fresh.borrow_mut().insert(id);
                            toasts(cx).update(cx, |t, cx| t.mark_read(id, cx));
                        }
                    });
                }
                let is_fresh = !row.note.read || fresh.borrow().contains(&id);
                render_row(row, (selected == Some(id)).then_some(action), is_fresh, view.clone(), now, &theme)
            })
            .flex_1()
            .min_h_0()
            .into_any_element()
        };
        div()
            .id("notifications")
            .debug_selector(|| "notifications".into())
            .key_context("Notifications")
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(t.bg)
            .on_action(cx.listener(|this, _: &NextNote, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PrevNote, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &NextAction, _, cx| this.step_action(1, cx)))
            .on_action(cx.listener(|this, _: &PrevAction, _, cx| this.step_action(-1, cx)))
            .on_action(cx.listener(|this, _: &RunNote, window, cx| this.run_selected(window, cx)))
            .on_action(cx.listener(|this, _: &DeleteNote, _, cx| this.delete_selected(cx)))
            .on_action(cx.listener(|this, _: &ToggleUnread, _, cx| this.set_unread_only(!this.unread_only, cx)))
            .on_action(cx.listener(|this, _: &MarkAllRead, _, cx| this.mark_all_read(cx)))
            .on_action(cx.listener(|this, _: &ClearNotes, _, cx| this.clear(cx)))
            .on_action(cx.listener(|this, _: &WorkspaceMenu, window, cx| this.toolbar(cx).open_menu_of("notes.workspace", window, cx)))
            .child(div().flex_none().border_b_1().border_color(t.line).child(toolbar))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_round_trip() {
        let a = open_action("View output", "work", &ViewRef("run/api/start".into()));
        assert_eq!(decode(&a.payload), Some(Act::Open { workspace: "work".into(), view: ViewRef("run/api/start".into()) }));
        assert_eq!(decode(&problems_action("View").payload), Some(Act::Problems));
        assert_eq!(decode("nope"), None);
        assert_eq!(decode("open"), None);
    }

    #[test]
    fn a_change_is_the_span_between_what_stays() {
        assert_eq!(changed_span(&[3, 2, 1], &[4, 3, 2, 1]), (0..0, 1), "one come in front");
        assert_eq!(changed_span(&[3, 2, 1], &[3, 1]), (1..2, 0), "one removed");
        assert_eq!(changed_span(&[3, 2, 1], &[3, 2, 1]), (3..3, 0));
        assert_eq!(changed_span(&[], &[2, 1]), (0..0, 2));
        assert_eq!(changed_span(&[1, 1], &[1]), (1..2, 0));
        assert_eq!(changed_span(&[5, 3, 2, 1], &[6, 5, 3, 1]), (0..3, 3), "several: the span between");
    }

    #[test]
    fn ages_in_words() {
        assert_eq!(ago(Duration::from_secs(5)), "just now");
        assert_eq!(ago(Duration::from_secs(125)), "2 min ago");
        assert_eq!(ago(Duration::from_secs(7300)), "2 h ago");
        assert_eq!(ago(Duration::from_secs(3 * 86400)), "3 d ago");
    }

    #[test]
    fn the_bell_names_its_counts() {
        assert_eq!(badge_words(Badge::Clear), "notifications");
        assert_eq!(badge_words(Badge::Unread(2)), "notifications: 2 unread");
        assert_eq!(badge_words(Badge::Errors { errors: 1, unread: 3 }), "notifications: 3 unread, 1 error");
    }
}
