//! Overlays drawn from a window's root, never inside a pane: context menus, a modal
//! stack and toasts. Each window has one [`OverlayLayer`]; the app-wide [`Toasts`] stack
//! is drawn by the main window.
//!
//! - Menus and modals sit on an occluding backdrop: an outside click closes them without
//!   reaching what's underneath.
//! - Each overlay has its own focus and key context (`Menu`, `Modal`), so pane keys never
//!   act behind it, and focus goes back where it was when it closes.
//! - Toasts expire on a single timer that stops when the stack is empty, so idle CPU
//!   stays at zero.

use crate::components::label;
use crate::theme::{Theme, theme};
use gpui::{
    Anchor, AnyElement, AnyView, AnyWindowHandle, App, AppContext, Context, Entity, FocusHandle, Global,
    Pixels, Point, Render, SharedString, Subscription, Task, WeakFocusHandle, Window, WindowId, actions, anchored,
    deferred, div, point, prelude::*,
};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Draw order of overlays (higher draws above).
pub const PRIORITY_MODAL: usize = 10;
pub const PRIORITY_MENU: usize = 20;
pub const PRIORITY_TOAST: usize = 30;

actions!(overlay, [MenuUp, MenuDown, MenuConfirm, MenuClose, ModalNext, ModalPrev, ModalClose]);

// ------------------------------------------------------------------ menu model

pub type MenuAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
pub struct MenuItem {
    pub label: SharedString,
    pub hint: Option<SharedString>,
    pub checked: bool,
    /// Why the item can't be used; shown inline when selected (R5.5).
    pub disabled: Option<SharedString>,
    pub separator: bool,
    pub run: Option<MenuAction>,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>, run: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { label: label.into(), hint: None, checked: false, disabled: None, separator: false, run: Some(Rc::new(run)) }
    }
    pub fn separator() -> Self {
        Self { label: "".into(), hint: None, checked: false, disabled: None, separator: true, run: None }
    }
    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.disabled = Some(reason.into());
        self
    }
    fn selectable(&self) -> bool {
        !self.separator
    }
}

/// The next selectable index from `from` in `dir` (±1), wrapping. Disabled items are
/// selectable (so their reason can be read) but don't run.
pub fn step_selection(items: &[MenuItem], from: Option<usize>, dir: isize) -> Option<usize> {
    let n = items.len() as isize;
    if n == 0 || !items.iter().any(MenuItem::selectable) {
        return None;
    }
    let mut i = from.map_or(if dir > 0 { -1 } else { n }, |f| f as isize);
    loop {
        i = (i + dir).rem_euclid(n);
        if items[i as usize].selectable() {
            return Some(i as usize);
        }
    }
}

// ------------------------------------------------------------------ menu view

pub struct Menu {
    items: Vec<MenuItem>,
    selected: Option<usize>,
    position: Point<Pixels>,
    focus: FocusHandle,
    restore: Option<WeakFocusHandle>,
    window: AnyWindowHandle,
    _blur: Option<Subscription>,
}

impl Menu {
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._blur = None;
        if let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            window.focus(&h, cx);
        }
        layer(self.window, cx).update(cx, |l, cx| {
            l.menu = None;
            cx.notify();
        });
    }

    fn run(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let item = self.items[i].clone();
        if item.disabled.is_some() || item.separator {
            self.selected = Some(i);
            cx.notify();
            return;
        }
        self.close(window, cx);
        if let Some(run) = item.run {
            run(window, cx);
        }
    }
}

impl Render for Menu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let viewport = window.viewport_size();
        let max_h = viewport.height - t.size(16.);
        let mut list = div()
            .id("menu")
            .key_context("Menu")
            .track_focus(&self.focus)
            .min_w(t.size(220.))
            .max_w(t.size(420.))
            .max_h(max_h)
            .overflow_y_scroll()
            .py_1()
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .rounded(t.radius_m)
            .text_size(t.text)
            .font_family(t.font.clone())
            .text_color(t.fg)
            .on_action(cx.listener(|this, _: &MenuDown, _, cx| {
                this.selected = step_selection(&this.items, this.selected, 1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MenuUp, _, cx| {
                this.selected = step_selection(&this.items, this.selected, -1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MenuConfirm, window, cx| {
                if let Some(i) = this.selected {
                    this.run(i, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MenuClose, window, cx| this.close(window, cx)));
        for (i, item) in self.items.iter().enumerate() {
            if item.separator {
                list = list.child(div().my_1().h(t.hairline).bg(t.line));
                continue;
            }
            let selected = self.selected == Some(i);
            let reason = item.disabled.clone().filter(|_| selected);
            let row = div()
                .id(("menu-item", i))
                .flex()
                .items_center()
                .gap_2()
                .h(t.row_h)
                .px_3()
                .when(selected, |d| d.bg(t.selected))
                .when(item.disabled.is_some(), |d| d.text_color(t.dim))
                .on_mouse_move(cx.listener(move |this, _, _, cx| {
                    if this.selected != Some(i) {
                        this.selected = Some(i);
                        cx.notify();
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| this.run(i, window, cx)))
                .child(div().w(t.size(14.)).flex_none().text_color(t.accent).child(if item.checked { "✓" } else { "" }))
                .child(label(item.label.clone()).flex_1())
                .when_some(reason, |d, r| d.child(label(r).text_color(t.warn).max_w(t.size(200.))))
                .when_some(item.hint.clone(), |d, h| d.child(div().flex_none().text_color(t.mute).text_size(t.text_small).child(h)));
            list = list.child(row);
        }
        let backdrop = div()
            .id("menu-backdrop")
            .w(viewport.width)
            .h(viewport.height)
            .occlude()
            .on_any_mouse_down(cx.listener(|this, _, window, cx| this.close(window, cx)));
        div()
            .child(deferred(anchored().position(point(Pixels::ZERO, Pixels::ZERO)).child(backdrop)).with_priority(PRIORITY_MENU - 1))
            .child(deferred(anchored().position(self.position).snap_to_window_with_margin(t.size(4.)).child(list)).with_priority(PRIORITY_MENU))
    }
}

// ------------------------------------------------------------------ modal

/// Focus handles of a modal's controls; tab and shift-tab cycle through them only.
#[derive(Clone, Default)]
pub struct ModalFocus(pub Vec<FocusHandle>);

impl ModalFocus {
    pub fn cycle(&self, dir: isize, window: &mut Window, cx: &mut App) {
        if self.0.is_empty() {
            return;
        }
        let current = self.0.iter().position(|h| h.contains_focused(window, cx)).map_or(-1, |i| i as isize);
        let next = (current + dir).rem_euclid(self.0.len() as isize) as usize;
        window.focus(&self.0[next], cx);
    }
}

/// Backdrop and centered panel for a modal's content.
pub fn modal_frame(content: AnyElement, window: &Window, t: &Theme) -> AnyElement {
    let viewport = window.viewport_size();
    deferred(
        anchored().position(point(Pixels::ZERO, Pixels::ZERO)).child(
            div()
                .id("modal-backdrop")
                .w(viewport.width)
                .h(viewport.height)
                .occlude()
                .bg(t.backdrop)
                .flex()
                .items_start()
                .justify_center()
                .pt(viewport.height * 0.15)
                .child(
                    div()
                        .min_w(t.size(420.))
                        .max_w(viewport.width - t.size(48.))
                        .bg(t.raised)
                        .border_1()
                        .border_color(t.line)
                        .rounded(t.radius_m)
                        .p_4()
                        .font_family(t.font.clone())
                        .text_size(t.text)
                        .text_color(t.fg)
                        .child(content),
                ),
        ),
    )
    .with_priority(PRIORITY_MODAL)
    .into_any_element()
}

// ------------------------------------------------------------------ per-window layer

/// What a window shows above its content: at most one menu and a stack of modals.
pub struct OverlayLayer {
    pub menu: Option<Entity<Menu>>,
    pub modals: Vec<AnyView>,
}

#[derive(Default)]
struct Layers(HashMap<WindowId, Entity<OverlayLayer>>);
impl Global for Layers {}

/// The overlay layer of a window, created on first use.
pub fn layer(window: AnyWindowHandle, cx: &mut App) -> Entity<OverlayLayer> {
    if !cx.has_global::<Layers>() {
        cx.set_global(Layers::default());
    }
    if let Some(l) = cx.global::<Layers>().0.get(&window.window_id()) {
        return l.clone();
    }
    let l = cx.new(|_| OverlayLayer { menu: None, modals: Vec::new() });
    cx.global_mut::<Layers>().0.insert(window.window_id(), l.clone());
    l
}

/// Drops a closed window's layer (call from `on_window_closed`).
pub fn forget_layer(window: WindowId, cx: &mut App) {
    if cx.has_global::<Layers>() {
        cx.global_mut::<Layers>().0.remove(&window);
    }
}

/// How many windows have an overlay layer (leak check).
pub fn layer_count(cx: &App) -> usize {
    cx.try_global::<Layers>().map_or(0, |l| l.0.len())
}

/// Opens a context menu at `position` (window coordinates) and moves focus into it.
pub fn open_menu(position: Point<Pixels>, items: Vec<MenuItem>, window: &mut Window, cx: &mut App) {
    let handle = window.window_handle();
    let restore = window.focused(cx).map(|f| f.downgrade());
    let selected = step_selection(&items, None, 1);
    let menu = cx.new(|cx| Menu {
        items,
        selected,
        position,
        focus: cx.focus_handle(),
        restore,
        window: handle,
        _blur: None,
    });
    let focus = menu.read(cx).focus.clone();
    window.focus(&focus, cx);
    // Opened from a mouse down, a focusable ancestor would take focus back as the event
    // bubbles on; this keeps it in the menu.
    window.prevent_default();
    menu.update(cx, |m, cx| {
        m._blur = Some(cx.on_blur(&focus, window, |this, window, cx| this.close(window, cx)));
    });
    layer(handle, cx).update(cx, |l, cx| {
        l.menu = Some(menu);
        cx.notify();
    });
}

pub fn push_modal(view: AnyView, window: &mut Window, cx: &mut App) {
    layer(window.window_handle(), cx).update(cx, |l, cx| {
        l.modals.push(view);
        cx.notify();
    });
}

/// Whether a window currently shows a menu, and how many modals.
pub fn layer_state(window: AnyWindowHandle, cx: &mut App) -> (bool, usize) {
    let l = layer(window, cx).read(cx);
    (l.menu.is_some(), l.modals.len())
}

pub fn pop_modal(window: &mut Window, cx: &mut App) {
    layer(window.window_handle(), cx).update(cx, |l, cx| {
        l.modals.pop();
        cx.notify();
    });
}

/// Elements to append to a window root: modals (each draws its own frame) and the menu.
pub fn render_layer(window: &Window, cx: &App) -> Vec<AnyElement> {
    let Some(l) = cx.try_global::<Layers>().and_then(|g| g.0.get(&window.window_handle().window_id())) else { return Vec::new() };
    let l = l.read(cx);
    let mut out: Vec<AnyElement> = l.modals.iter().map(|m| m.clone().into_any_element()).collect();
    if let Some(m) = &l.menu {
        out.push(m.clone().into_any_element());
    }
    out
}

// ------------------------------------------------------------------ toasts

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Ok,
    Warn,
    Err,
}

#[derive(Clone)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub text: SharedString,
    pub actions: Vec<(SharedString, MenuAction)>,
    pub deadline: Instant,
}

pub fn toast_lifetime(kind: ToastKind) -> Duration {
    if kind == ToastKind::Err { Duration::from_secs(10) } else { Duration::from_secs(6) }
}

/// Pure part of the stack: expire by time, keep at most `max`.
#[derive(Default)]
pub struct ToastQueue {
    pub items: Vec<Toast>,
    next_id: u64,
}

impl ToastQueue {
    pub const MAX: usize = 4;

    pub fn push(&mut self, kind: ToastKind, text: SharedString, actions: Vec<(SharedString, MenuAction)>, now: Instant) -> u64 {
        self.next_id += 1;
        self.items.push(Toast { id: self.next_id, kind, text, actions, deadline: now + toast_lifetime(kind) });
        if self.items.len() > Self::MAX {
            self.items.remove(0);
        }
        self.next_id
    }

    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }

    /// Drops expired toasts; returns the next deadline, if any.
    pub fn expire(&mut self, now: Instant) -> Option<Instant> {
        self.items.retain(|t| t.deadline > now);
        self.items.iter().map(|t| t.deadline).min()
    }
}

pub struct Toasts {
    pub queue: ToastQueue,
    timer: Option<Task<()>>,
}

struct ToastsHandle(Entity<Toasts>);
impl Global for ToastsHandle {}

pub fn toasts(cx: &mut App) -> Entity<Toasts> {
    if let Some(h) = cx.try_global::<ToastsHandle>() {
        return h.0.clone();
    }
    let e = cx.new(|_| Toasts { queue: ToastQueue::default(), timer: None });
    cx.set_global(ToastsHandle(e.clone()));
    e
}

impl Toasts {
    pub fn push(&mut self, kind: ToastKind, text: impl Into<SharedString>, actions: Vec<(SharedString, MenuAction)>, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        self.queue.push(kind, text.into(), actions, now);
        self.arm(cx);
        cx.notify();
    }

    pub fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.queue.dismiss(id);
        self.arm(cx);
        cx.notify();
    }

    pub fn timer_running(&self) -> bool {
        self.timer.is_some()
    }

    /// One timer, sleeping until the earliest deadline; none when the stack is empty.
    fn arm(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let Some(next) = self.queue.expire(now) else {
            self.timer = None;
            return;
        };
        let wait = next.saturating_duration_since(now);
        self.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |t, cx| {
                t.timer = None;
                t.arm(cx);
                cx.notify();
            });
        }));
    }
}

impl Render for Toasts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        if self.queue.items.is_empty() {
            return div().into_any_element();
        }
        let viewport = window.viewport_size();
        let mut stack = div().flex().flex_col().items_end().gap_2().font_family(t.font.clone()).text_size(t.text);
        for toast in self.queue.items.clone() {
            let (glyph, color, border) = match toast.kind {
                ToastKind::Info => ("●", t.accent, t.line),
                ToastKind::Ok => ("✓", t.ok, t.line),
                ToastKind::Warn => ("⚠", t.warn, t.warn),
                ToastKind::Err => ("✖", t.err, t.err),
            };
            let id = toast.id;
            let mut row = div()
                .id(("toast", id as usize))
                .flex()
                .items_center()
                .gap_3()
                .max_w(t.size(640.))
                .px_3()
                .py_1()
                .bg(t.raised)
                .border_1()
                .border_color(border)
                .rounded(t.radius_m)
                .occlude()
                .child(div().flex_none().text_color(color).child(glyph))
                .child(label(toast.text.clone()).text_color(t.fg));
            for (i, (name, run)) in toast.actions.iter().enumerate() {
                let run = run.clone();
                row = row.child(
                    crate::components::button(("toast-action", (id as usize) * 16 + i), name.clone(), &t).on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.dismiss(id, cx);
                            run(window, cx);
                        },
                    )),
                );
            }
            row = row.child(
                crate::components::button(("toast-close", id as usize), "×", &t)
                    .on_click(cx.listener(move |this, _, _, cx| this.dismiss(id, cx))),
            );
            stack = stack.child(row);
        }
        let margin = t.size(16.);
        deferred(
            anchored()
                .anchor(Anchor::BottomRight)
                .position(point(viewport.width - margin, viewport.height - margin))
                .child(stack),
        )
        .with_priority(PRIORITY_TOAST)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> MenuItem {
        MenuItem::new(name.to_string(), |_, _| {})
    }

    #[test]
    fn menu_selection_skips_separators_and_wraps() {
        let items = vec![item("a"), MenuItem::separator(), item("b").disabled("no"), item("c")];
        assert_eq!(step_selection(&items, None, 1), Some(0));
        assert_eq!(step_selection(&items, Some(0), 1), Some(2), "disabled stays selectable to show why");
        assert_eq!(step_selection(&items, Some(3), 1), Some(0), "wraps");
        assert_eq!(step_selection(&items, Some(0), -1), Some(3));
        assert_eq!(step_selection(&items, None, -1), Some(3));
        assert_eq!(step_selection(&[MenuItem::separator()], None, 1), None);
        assert_eq!(step_selection(&[], None, 1), None);
    }

    #[test]
    fn toast_queue_expiry_and_cap() {
        let t0 = Instant::now();
        let mut q = ToastQueue::default();
        q.push(ToastKind::Info, "a".into(), vec![], t0);
        q.push(ToastKind::Err, "b".into(), vec![], t0);
        assert_eq!(q.expire(t0), Some(t0 + Duration::from_secs(6)));
        assert_eq!(q.expire(t0 + Duration::from_secs(7)), Some(t0 + Duration::from_secs(10)));
        assert_eq!(q.items.len(), 1, "info expired, error stays longer");
        assert_eq!(q.expire(t0 + Duration::from_secs(11)), None);
        for i in 0..6 {
            q.push(ToastKind::Info, format!("{i}").into(), vec![], t0);
        }
        assert_eq!(q.items.len(), ToastQueue::MAX);
        assert_eq!(q.items[0].text.as_ref(), "2", "oldest dropped first");
        let id = q.items[1].id;
        q.dismiss(id);
        assert_eq!(q.items.len(), ToastQueue::MAX - 1);
    }
}
