//! Overlays drawn from a window's root, never inside a pane: context menus, a modal
//! stack and toasts. Each window has one [`OverlayLayer`]; the app-wide [`Toasts`] stack
//! is drawn by the main window.
//!
//! - Menus and modals sit on an occluding backdrop: an outside click closes them without
//!   reaching what's underneath.
//! - A menu item can carry a submenu (where to do it): a click does the item's default,
//!   resting the pointer on it for [`SUBMENU_DELAY`] (or →) opens the submenu beside it;
//!   ↑↓ and Enter then work in it, ← or Esc close it.
//! - A toggle (on or off) keeps the menu open when used, its check flipped, so several
//!   can be changed in a row (R5.8); a pick (one of several) and an action close it.
//! - Each overlay has its own focus and key context (`Menu`, `Modal`), so pane keys never
//!   act behind it, and focus goes back where it was when it closes.
//! - Toasts expire on a single timer that stops when nothing shown runs, so idle CPU
//!   stays at zero. Three show at once, the others wait their turn ("+N more"); the
//!   pointer resting on one pauses it. Every toast is also kept as a [`Note`] in the
//!   notification center's [`NoteStore`], beside the control API's history.

use crate::scroll::Scrolls;
use crate::components::label;
use crate::icons::{Icon, icon};
use crate::theme::{Theme, theme};
use gpui::{
    Anchor, AnyElement, AnyView, AnyWindowHandle, App, AppContext, Context, Entity, FocusHandle, Global, Hsla,
    Pixels, Point, Render, ScrollHandle, SharedString, Subscription, Task, WeakFocusHandle, Window, WindowId, actions, anchored,
    deferred, div, point, prelude::*, px,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Draw order of overlays (higher draws above).
pub const PRIORITY_MODAL: usize = 10;
pub const PRIORITY_MENU: usize = 20;
pub const PRIORITY_TOAST: usize = 30;

actions!(overlay, [MenuUp, MenuDown, MenuConfirm, MenuClose, MenuOpenSub, MenuCloseSub, ModalNext, ModalPrev, ModalClose]);

/// How long the pointer rests on an item before its submenu opens.
pub const SUBMENU_DELAY: Duration = Duration::from_millis(300);

// ------------------------------------------------------------------ menu model

pub type MenuAction = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
pub struct MenuItem {
    pub label: SharedString,
    pub hint: Option<SharedString>,
    pub checked: bool,
    /// Turns something on or off: using it keeps the menu open (R5.8).
    pub toggle: bool,
    /// Why the item can't be used. Never drawn: a disabled row is only dimmed, so selecting
    /// it doesn't widen the menu (R5.5, R9.10).
    pub disabled: Option<SharedString>,
    pub separator: bool,
    /// Shown before the label (a protected env's lock…).
    pub icon: Option<(Icon, Hsla)>,
    pub run: Option<MenuAction>,
    /// Shown beside it on rest or →: other ways to do it (other places).
    pub submenu: Vec<MenuItem>,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>, run: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { label: label.into(), hint: None, checked: false, toggle: false, disabled: None, separator: false, icon: None, run: Some(Rc::new(run)), submenu: Vec::new() }
    }
    pub fn separator() -> Self {
        Self { label: "".into(), hint: None, checked: false, toggle: false, disabled: None, separator: true, icon: None, run: None, submenu: Vec::new() }
    }
    /// Other ways to do it, in a submenu; clicking the item itself still does its own.
    pub fn submenu(mut self, items: Vec<MenuItem>) -> Self {
        self.submenu = items;
        self
    }
    pub fn icon(mut self, icon: Icon, color: Hsla) -> Self {
        self.icon = Some((icon, color));
        self
    }
    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    /// The one picked of several: using it closes the menu.
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    /// A toggle, checked when on: using it flips its check and keeps the menu open, so
    /// several can be changed in a row (R5.8).
    pub fn toggle(mut self, on: bool) -> Self {
        self.checked = on;
        self.toggle = true;
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
/// selectable but don't run.
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
    /// The list's scroll, so the row the keys select stays in view.
    scroll: ScrollHandle,
    /// The open submenu: whose it is, and its selection.
    sub: Option<(usize, Option<usize>)>,
    /// Where each row was drawn last: a submenu opens beside its row.
    rows: Rc<RefCell<BTreeMap<usize, gpui::Bounds<Pixels>>>>,
    /// The pointer resting on a row with a submenu, until [`SUBMENU_DELAY`].
    resting: Option<Task<()>>,
    /// The row under the pointer: only entering a row acts, so a jitter on a row whose
    /// submenu was just closed with ← or Esc doesn't open it again.
    pointer: Option<usize>,
    _blur: Option<Subscription>,
}

impl Menu {
    /// What it offers.
    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    /// Removes the menu. Focus goes back to what had it on open, unless it already moved
    /// elsewhere (`give_back_focus` false), which then keeps it.
    fn close(&mut self, give_back_focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        self._blur = None;
        if give_back_focus && let Some(h) = self.restore.as_ref().and_then(WeakFocusHandle::upgrade) {
            window.focus(&h, cx);
        }
        layer(self.window, cx).update(cx, |l, cx| {
            l.menu = None;
            cx.notify();
        });
    }

    /// Selects the next selectable row in `dir` and scrolls it into view (each item is one
    /// child of the list, so its index is the child's).
    fn step(&mut self, dir: isize, cx: &mut Context<Self>) {
        self.selected = step_selection(&self.items, self.selected, dir);
        if let Some(i) = self.selected {
            self.scroll.scroll_to_item(i);
        }
        cx.notify();
    }

    /// The pointer is on row `i`: it's selected, and after [`SUBMENU_DELAY`] there its
    /// submenu opens (or another row's closes). The open one stays meanwhile, so the
    /// pointer can cross rows on its way to it.
    fn hover(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.pointer.replace(i) == Some(i) {
            return;
        }
        let parent = self.sub.map(|(p, _)| p);
        self.selected = Some(i);
        self.resting = (parent != Some(i) && (parent.is_some() || !self.items[i].submenu.is_empty())).then(|| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SUBMENU_DELAY).await;
                let _ = this.update(cx, |m, cx| {
                    m.resting = None;
                    if m.selected == Some(i) {
                        m.sub = None;
                        m.open_sub(i, cx);
                        cx.notify();
                    }
                });
            })
        });
        cx.notify();
    }

    /// The pointer is on submenu item `j`: it's selected, and its row stays the menu's.
    fn hover_sub(&mut self, j: usize, cx: &mut Context<Self>) {
        let Some((p, s)) = self.sub else { return };
        self.pointer = None;
        if s == Some(j) && self.selected == Some(p) && self.resting.is_none() {
            return;
        }
        self.sub = Some((p, Some(j)));
        self.selected = Some(p);
        self.resting = None;
        cx.notify();
    }

    /// Opens row `i`'s submenu, its first item selected. Not before the row was drawn
    /// (it opens beside it), so keys never act in a submenu that isn't shown.
    fn open_sub(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(i).filter(|it| !it.submenu.is_empty() && it.disabled.is_none()) else { return };
        if !self.rows.borrow().contains_key(&i) {
            return;
        }
        self.resting = None;
        self.sub = Some((i, step_selection(&item.submenu, None, 1)));
        cx.notify();
    }

    /// A key wins over the pointer resting somewhere: the rest is cancelled, and with a
    /// submenu open its row is the selected one again (the keys act in the submenu).
    fn take_keys(&mut self) {
        self.resting = None;
        if let Some((p, _)) = self.sub {
            self.selected = Some(p);
        }
    }

    fn run(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let item = self.items[i].clone();
        if item.disabled.is_some() || item.separator {
            self.selected = Some(i);
            cx.notify();
            return;
        }
        if item.toggle {
            self.selected = Some(i);
            self.items[i].checked = !item.checked;
            cx.notify();
            if let Some(run) = item.run {
                run(window, cx);
            }
            return;
        }
        self.close(true, window, cx);
        if let Some(run) = item.run {
            run(window, cx);
        }
    }

    /// Runs item `j` of the open submenu.
    fn run_sub(&mut self, j: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((parent, _)) = self.sub else { return };
        let Some(item) = self.items[parent].submenu.get(j).cloned() else { return };
        if item.disabled.is_some() || item.separator {
            self.sub = Some((parent, Some(j)));
            cx.notify();
            return;
        }
        if item.toggle {
            self.sub = Some((parent, Some(j)));
            self.items[parent].submenu[j].checked = !item.checked;
            cx.notify();
            if let Some(run) = item.run {
                run(window, cx);
            }
            return;
        }
        self.close(true, window, cx);
        if let Some(run) = item.run {
            run(window, cx);
        }
    }

    /// The keys' row moves in the submenu when one is open, else in the menu.
    fn step_any(&mut self, dir: isize, cx: &mut Context<Self>) {
        match self.sub {
            Some((p, sel)) => {
                self.sub = Some((p, step_selection(&self.items[p].submenu, sel, dir)));
                cx.notify();
            }
            None => self.step(dir, cx),
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
            .track_scroll(&self.scroll)
            .min_w(t.size(220.))
            .max_w(t.size(420.))
            .max_h(max_h)
            .scroll_y()
            .py_1()
            .bg(t.raised)
            .border_1()
            .border_color(t.line)
            .rounded(t.radius_m)
            .text_size(t.text)
            .font_family(t.font.clone())
            .text_color(t.fg)
            // The backdrop under the list closes the menu on mouse down: it must not get the
            // clicks meant for rows, or the menu closes before a row's click completes.
            .occlude()
            .on_action(cx.listener(|this, _: &MenuDown, _, cx| {
                this.take_keys();
                this.step_any(1, cx)
            }))
            .on_action(cx.listener(|this, _: &MenuUp, _, cx| {
                this.take_keys();
                this.step_any(-1, cx)
            }))
            .on_action(cx.listener(|this, _: &MenuConfirm, window, cx| {
                this.take_keys();
                match this.sub {
                    Some((_, Some(j))) => this.run_sub(j, window, cx),
                    Some((_, None)) => {}
                    None => {
                        if let Some(i) = this.selected {
                            this.run(i, window, cx);
                        }
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &MenuOpenSub, _, cx| {
                this.take_keys();
                if let Some(i) = this.selected {
                    this.open_sub(i, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MenuCloseSub, _, cx| {
                this.take_keys();
                this.sub = None;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &MenuClose, window, cx| {
                this.take_keys();
                if this.sub.take().is_some() {
                    cx.notify();
                } else {
                    this.close(true, window, cx);
                }
            }));
        for (i, item) in self.items.iter().enumerate() {
            if item.separator {
                list = list.child(div().my_1().h(t.hairline).bg(t.line));
                continue;
            }
            let selected = self.selected == Some(i);
            let rows = self.rows.clone();
            let row = div()
                .id(("menu-item", i))
                .debug_selector(move || format!("menu-item-{i}"))
                .relative()
                .flex()
                .items_center()
                .gap_2()
                .h(t.row_h)
                .px_3()
                .when(selected, |d| d.bg(t.selected))
                .when(item.disabled.is_some(), |d| d.text_color(t.dim))
                .on_mouse_move(cx.listener(move |this, _, _, cx| this.hover(i, cx)))
                .on_click(cx.listener(move |this, _, window, cx| this.run(i, window, cx)))
                .child(div().size(t.icon).flex_none().when(item.checked, |d| d.child(icon(Icon::Check, if item.disabled.is_some() { t.dim } else { t.accent }, &t))))
                .when_some(item.icon, |d, (i, c)| d.child(icon(i, if item.disabled.is_some() { t.dim } else { c }, &t)))
                .child(label(item.label.clone()).flex_1())
                .when_some(item.hint.clone(), |d, h| d.child(div().flex_none().text_color(t.mute).text_size(t.text_small).child(h)))
                .when(!item.submenu.is_empty(), |d| d.child(icon(Icon::Submenu, t.mute, &t)))
                .child(gpui::canvas(move |b, _, _| rows.borrow_mut().insert(i, b), |_, _, _, _| {}).absolute().top_0().left_0().size_full());
            list = list.child(row);
        }
        // The open submenu, beside its row.
        // Its first item level with the row: both lists have the same border and padding.
        let inset = px(1.) + window.rem_size() * 0.25;
        let submenu = self.sub.and_then(|(parent, sel)| {
            let at = self.rows.borrow().get(&parent).map(|b| point(b.right(), b.top() - inset))?;
            let mut sub = div()
                .id("submenu")
                .min_w(t.size(200.))
                .max_w(t.size(420.))
                .py_1()
                .bg(t.raised)
                .border_1()
                .border_color(t.line)
                .rounded(t.radius_m)
                .text_size(t.text)
                .font_family(t.font.clone())
                .text_color(t.fg)
                .occlude();
            for (j, item) in self.items[parent].submenu.iter().enumerate() {
                if item.separator {
                    sub = sub.child(div().my_1().h(t.hairline).bg(t.line));
                    continue;
                }
                let selected = sel == Some(j);
                sub = sub.child(
                    div()
                        .id(("submenu-item", j))
                        .debug_selector(move || format!("submenu-item-{j}"))
                        .flex()
                        .items_center()
                        .gap_2()
                        .h(t.row_h)
                        .px_3()
                        .when(selected, |d| d.bg(t.selected))
                        .when(item.disabled.is_some(), |d| d.text_color(t.dim))
                        .on_mouse_move(cx.listener(move |this, _, _, cx| this.hover_sub(j, cx)))
                        .on_click(cx.listener(move |this, _, window, cx| this.run_sub(j, window, cx)))
                        .child(div().size(t.icon).flex_none().when(item.checked, |d| d.child(icon(Icon::Check, if item.disabled.is_some() { t.dim } else { t.accent }, &t))))
                        .when_some(item.icon, |d, (i, c)| d.child(icon(i, if item.disabled.is_some() { t.dim } else { c }, &t)))
                        .child(label(item.label.clone()).flex_1())
                        .when_some(item.hint.clone(), |d, h| d.child(div().flex_none().text_color(t.mute).text_size(t.text_small).child(h))),
                );
            }
            Some((at, sub))
        });
        let backdrop = div()
            .id("menu-backdrop")
            .w(viewport.width)
            .h(viewport.height)
            .occlude()
            .on_any_mouse_down(cx.listener(|this, _, window, cx| this.close(true, window, cx)));
        div()
            .child(deferred(anchored().position(point(Pixels::ZERO, Pixels::ZERO)).child(backdrop)).with_priority(PRIORITY_MENU - 1))
            .child(deferred(anchored().position(self.position).snap_to_window_with_margin(t.size(4.)).child(list)).with_priority(PRIORITY_MENU))
            .children(submenu.map(|(at, sub)| deferred(anchored().position(at).snap_to_window_with_margin(t.size(4.)).child(sub)).with_priority(PRIORITY_MENU + 1)))
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
        let n = self.0.len() as isize;
        // From outside the controls, forward starts at the first and backward at the last.
        let current = self.0.iter().position(|h| h.contains_focused(window, cx)).map_or(if dir > 0 { -1 } else { n }, |i| i as isize);
        let next = (current + dir).rem_euclid(n) as usize;
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
    // A menu already open closes first, so focus goes back to what had it before both
    // rather than to the replaced menu.
    let overlays = layer(handle, cx);
    if let Some(open) = overlays.read(cx).menu.clone() {
        open.update(cx, |m, cx| m.close(true, window, cx));
    }
    let restore = window.focused(cx).map(|f| f.downgrade());
    let selected = step_selection(&items, None, 1);
    let menu = cx.new(|cx| Menu {
        items,
        selected,
        position,
        focus: cx.focus_handle(),
        restore,
        window: handle,
        scroll: ScrollHandle::new(),
        sub: None,
        rows: Rc::default(),
        resting: None,
        pointer: None,
        _blur: None,
    });
    let focus = menu.read(cx).focus.clone();
    window.focus(&focus, cx);
    // Opened from a mouse down, a focusable ancestor would take focus back as the event
    // bubbles on; this keeps it in the menu.
    window.prevent_default();
    menu.update(cx, |m, cx| {
        // Focus moved elsewhere on purpose (a pane, a modal): close and leave it there.
        m._blur = Some(cx.on_blur(&focus, window, |this, window, cx| this.close(false, window, cx)));
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

/// An action kept as data, so the notification center can offer it long after the toast
/// left: a label and a payload only the app reads, resolved to what it does at the click
/// ([`Toasts::set_hooks`]). Nothing is captured, so nothing is kept alive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteAction {
    pub label: SharedString,
    pub payload: SharedString,
}

impl NoteAction {
    pub fn new(label: impl Into<SharedString>, payload: impl Into<SharedString>) -> Self {
        Self { label: label.into(), payload: payload.into() }
    }
}

#[derive(Clone)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    /// One line above the text.
    pub title: Option<SharedString>,
    pub text: SharedString,
    /// Actions as closures: on the toast only, never in the center.
    pub actions: Vec<(SharedString, MenuAction)>,
    /// Actions as data: on the toast and in the center.
    pub notes: Vec<NoteAction>,
    pub lifetime: Duration,
    /// When it leaves; `None` while it waits for a place ([`ToastQueue::SHOWN`]): its
    /// lifetime starts when it shows.
    pub deadline: Option<Instant>,
    /// The time it had left when the pointer came to rest on it: its timer is paused.
    pub paused: Option<Duration>,
}

/// How long a toast stays once shown: errors longer.
pub fn toast_lifetime(kind: ToastKind) -> Duration {
    if kind == ToastKind::Err { Duration::from_secs(12) } else { Duration::from_secs(8) }
}

/// What a toast says, before it has an id: [`Toasts::push_new`].
#[derive(Clone)]
pub struct NewToast {
    pub kind: ToastKind,
    pub title: Option<SharedString>,
    pub text: SharedString,
    /// Whose it is, for the center (the app passes a workspace's id); `None`: the app's.
    pub source: Option<SharedString>,
    pub actions: Vec<(SharedString, MenuAction)>,
    pub notes: Vec<NoteAction>,
}

impl NewToast {
    pub fn new(kind: ToastKind, text: impl Into<SharedString>) -> Self {
        Self { kind, title: None, text: text.into(), source: None, actions: Vec::new(), notes: Vec::new() }
    }
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn source(mut self, source: Option<SharedString>) -> Self {
        self.source = source;
        self
    }
    /// Actions as closures (on the toast only).
    pub fn actions(mut self, actions: Vec<(SharedString, MenuAction)>) -> Self {
        self.actions = actions;
        self
    }
    /// An action as data (on the toast and in the center).
    pub fn note(mut self, action: NoteAction) -> Self {
        self.notes.push(action);
        self
    }
}

/// Pure part of the stack: the live toasts (the first [`ToastQueue::SHOWN`] shown, the
/// others waiting their turn), expiry by time, and the history.
#[derive(Default)]
pub struct ToastQueue {
    pub items: Vec<Toast>,
    /// The last toasts shown, expired or not, oldest first (time order, not alphabetical:
    /// what was said and when, for the control API's `ui.toasts`).
    pub history: std::collections::VecDeque<Told>,
    /// The titles of the toasts in `history`, by id (`Told` is the API's and stays as it
    /// is; a title reaches the API from here).
    pub titles: BTreeMap<u64, SharedString>,
    next_id: u64,
}

/// A toast as it was shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Told {
    pub id: u64,
    pub kind: ToastKind,
    pub text: SharedString,
}

impl ToastQueue {
    /// Toasts shown at once; the others wait ("+N more").
    pub const SHOWN: usize = 3;
    /// Live toasts kept, shown or waiting: past it the oldest waiting one goes (it stays
    /// in the center).
    pub const LIVE: usize = 100;
    /// Toasts kept in the history.
    pub const HISTORY: usize = 50;

    /// Shows a toast; returns its id.
    pub fn push(&mut self, kind: ToastKind, text: SharedString, actions: Vec<(SharedString, MenuAction)>, now: Instant) -> u64 {
        self.push_new(NewToast::new(kind, text).actions(actions), now)
    }

    /// Shows a toast (or queues it behind the shown ones); returns its id.
    pub fn push_new(&mut self, t: NewToast, now: Instant) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        if self.history.len() == Self::HISTORY
            && let Some(gone) = self.history.pop_front()
        {
            self.titles.remove(&gone.id);
        }
        self.history.push_back(Told { id, kind: t.kind, text: t.text.clone() });
        if let Some(title) = &t.title {
            self.titles.insert(id, title.clone());
        }
        self.items.push(Toast {
            id,
            kind: t.kind,
            title: t.title,
            text: t.text,
            actions: t.actions,
            notes: t.notes,
            lifetime: toast_lifetime(t.kind),
            deadline: None,
            paused: None,
        });
        if self.items.len() > Self::LIVE {
            self.items.remove(Self::SHOWN);
        }
        self.promote(now);
        id
    }

    /// The toasts on screen.
    pub fn shown(&self) -> &[Toast] {
        &self.items[..self.items.len().min(Self::SHOWN)]
    }

    /// How many wait behind them.
    pub fn waiting(&self) -> usize {
        self.items.len().saturating_sub(Self::SHOWN)
    }

    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|t| t.id != id);
    }

    /// The pointer rests on toast `id`: its timer stops with the time it has left.
    pub fn pause(&mut self, id: u64, now: Instant) {
        if let Some(t) = self.items.iter_mut().find(|t| t.id == id && t.paused.is_none())
            && let Some(d) = t.deadline
        {
            t.paused = Some(d.saturating_duration_since(now));
        }
    }

    /// The pointer left toast `id`: its timer goes on with the time it had left.
    pub fn resume(&mut self, id: u64, now: Instant) {
        if let Some(t) = self.items.iter_mut().find(|t| t.id == id)
            && let Some(left) = t.paused.take()
        {
            t.deadline = Some(now + left);
        }
    }

    /// Toasts that get a place start their lifetime.
    fn promote(&mut self, now: Instant) {
        for t in self.items.iter_mut().take(Self::SHOWN) {
            if t.deadline.is_none() {
                t.deadline = Some(now + t.lifetime);
            }
        }
    }

    /// Drops expired toasts, gives their places to waiting ones; returns the next
    /// deadline, if any (paused toasts have none).
    pub fn expire(&mut self, now: Instant) -> Option<Instant> {
        self.items.retain(|t| t.paused.is_some() || t.deadline.is_none_or(|d| d > now));
        self.promote(now);
        self.items.iter().filter(|t| t.paused.is_none()).filter_map(|t| t.deadline).min()
    }
}

/// A toast kept in the notification center.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// The toast's id (the same in `history`).
    pub id: u64,
    pub kind: ToastKind,
    pub title: Option<SharedString>,
    pub text: SharedString,
    /// When it came (the app's clock).
    pub at: Instant,
    /// Whose it is: opaque here (the app passes a workspace's id); `None`: the app's.
    pub source: Option<SharedString>,
    /// Its actions as data; a toast's closures aren't kept.
    pub actions: Vec<NoteAction>,
    pub read: bool,
}

/// The notification center's store: every toast, newest first, the last
/// [`NoteStore::MAX`]. Beside the history, which it never touches: clearing it leaves
/// the control API's `ui.toasts` as it was.
#[derive(Default)]
pub struct NoteStore {
    notes: std::collections::VecDeque<Note>,
}

impl NoteStore {
    pub const MAX: usize = 500;

    pub fn add(&mut self, note: Note) {
        self.notes.push_front(note);
        self.notes.truncate(Self::MAX);
    }

    /// Newest first.
    pub fn notes(&self) -> &std::collections::VecDeque<Note> {
        &self.notes
    }

    pub fn get(&self, id: u64) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    pub fn len(&self) -> usize {
        self.notes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Whether it changed.
    pub fn mark_read(&mut self, id: u64) -> bool {
        match self.notes.iter_mut().find(|n| n.id == id && !n.read) {
            Some(n) => {
                n.read = true;
                true
            }
            None => false,
        }
    }

    /// Marks read every note `which` picks; how many changed.
    pub fn mark_read_where(&mut self, which: impl Fn(&Note) -> bool) -> usize {
        let mut n = 0;
        for note in self.notes.iter_mut().filter(|x| !x.read && which(x)) {
            note.read = true;
            n += 1;
        }
        n
    }

    /// Whether it was there.
    pub fn delete(&mut self, id: u64) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        self.notes.len() != before
    }

    /// Removes every note `which` picks.
    pub fn delete_where(&mut self, which: impl Fn(&Note) -> bool) {
        self.notes.retain(|n| !which(n));
    }

    pub fn clear(&mut self) {
        self.notes.clear();
    }

    pub fn unread(&self) -> usize {
        self.notes.iter().filter(|n| !n.read).count()
    }

    /// Unread errors (warnings never count: they are mostly the user's own action
    /// refused).
    pub fn unread_errors(&self) -> usize {
        self.notes.iter().filter(|n| !n.read && n.kind == ToastKind::Err).count()
    }
}

/// Opens the notification center, on a notification or not (the app's).
pub type OpenCenter = Rc<dyn Fn(Option<u64>, &mut Window, &mut App)>;
/// Does what a [`NoteAction`] says (the app's).
pub type ResolveNote = Rc<dyn Fn(&NoteAction, &mut Window, &mut App)>;

pub struct Toasts {
    pub queue: ToastQueue,
    /// The notification center's store.
    pub notes: NoteStore,
    timer: Option<Task<()>>,
    /// The toast under the pointer (its timer paused).
    hovered: Option<u64>,
    open_center: Option<OpenCenter>,
    resolve: Option<ResolveNote>,
}

struct ToastsHandle(Entity<Toasts>);
impl Global for ToastsHandle {}

pub fn toasts(cx: &mut App) -> Entity<Toasts> {
    if let Some(h) = cx.try_global::<ToastsHandle>() {
        return h.0.clone();
    }
    let e = cx.new(|_| Toasts { queue: ToastQueue::default(), notes: NoteStore::default(), timer: None, hovered: None, open_center: None, resolve: None });
    cx.set_global(ToastsHandle(e.clone()));
    e
}

impl Toasts {
    /// Shows a toast, the app's (no source); returns its id.
    pub fn push(&mut self, kind: ToastKind, text: impl Into<SharedString>, actions: Vec<(SharedString, MenuAction)>, cx: &mut Context<Self>) -> u64 {
        self.push_new(NewToast::new(kind, text).actions(actions), cx)
    }

    /// Shows a toast of `source` (a background workspace's: the screen's may be another).
    pub fn push_in(
        &mut self,
        source: Option<SharedString>,
        kind: ToastKind,
        text: impl Into<SharedString>,
        actions: Vec<(SharedString, MenuAction)>,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.push_new(NewToast::new(kind, text).source(source).actions(actions), cx)
    }

    /// Shows a toast and keeps it in the center; returns its id.
    pub fn push_new(&mut self, t: NewToast, cx: &mut Context<Self>) -> u64 {
        let now = cx.background_executor().now();
        let note = Note { id: 0, kind: t.kind, title: t.title.clone(), text: t.text.clone(), at: now, source: t.source.clone(), actions: t.notes.clone(), read: false };
        let id = self.queue.push_new(t, now);
        self.notes.add(Note { id, ..note });
        self.arm(cx);
        cx.notify();
        id
    }

    /// Takes the toast off the screen (it stays in the center as it was).
    pub fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.queue.dismiss(id);
        if self.hovered == Some(id) {
            self.hovered = None;
        }
        self.arm(cx);
        cx.notify();
    }

    /// The user closed it: off the screen, and read.
    pub fn close(&mut self, id: u64, cx: &mut Context<Self>) {
        self.notes.mark_read(id);
        self.dismiss(id, cx);
    }

    pub fn mark_read(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.notes.mark_read(id) {
            cx.notify();
        }
    }

    /// How the app opens its center and does what a [`NoteAction`] says.
    pub fn set_hooks(&mut self, open_center: OpenCenter, resolve: ResolveNote) {
        self.open_center = Some(open_center);
        self.resolve = Some(resolve);
    }

    /// Does what `action` says (the app's hook); nothing without one.
    pub fn resolver(&self) -> Option<ResolveNote> {
        self.resolve.clone()
    }

    pub fn timer_running(&self) -> bool {
        self.timer.is_some()
    }

    /// The pointer came to rest on toast `id`, or left it.
    fn hover(&mut self, id: u64, on: bool, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        if on {
            if let Some(old) = self.hovered.replace(id).filter(|old| *old != id) {
                self.queue.resume(old, now);
            }
            self.queue.pause(id, now);
        } else if self.hovered == Some(id) {
            self.hovered = None;
            self.queue.resume(id, now);
        }
        self.arm(cx);
        cx.notify();
    }

    /// One timer, sleeping until the earliest deadline; none when nothing shown runs.
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

    /// Opens the center (on `id`), through the app's hook.
    fn open(&mut self, id: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = id {
            self.dismiss(id, cx);
        }
        if let Some(open) = self.open_center.clone() {
            open(id, window, cx);
        }
    }
}

/// A toast's icon and color, and its border.
pub fn kind_look(kind: ToastKind, t: &Theme) -> (Icon, Hsla, Hsla) {
    match kind {
        ToastKind::Info => (Icon::Info, t.accent, t.line),
        ToastKind::Ok => (Icon::Ok, t.ok, t.line),
        ToastKind::Warn => (Icon::Warning, t.warn, t.warn),
        ToastKind::Err => (Icon::Error, t.err, t.err),
    }
}

/// Lines of a toast's text before it is cut (the rest is in the center): text wraps
/// there, an opt-in of R4.4.
pub const TOAST_LINES: usize = 3;

impl Render for Toasts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        if self.queue.items.is_empty() {
            return div().into_any_element();
        }
        let viewport = window.viewport_size();
        let margin = t.size(16.);
        let width = t.size(400.).min(viewport.width - margin * 2.);
        let mut stack = div().flex().flex_col().items_end().gap_2().font_family(t.font.clone()).text_size(t.text);
        for toast in self.queue.shown().to_vec() {
            let (kind, color, border) = kind_look(toast.kind, &t);
            let id = toast.id;
            let mut buttons = div().flex().flex_wrap().gap_1();
            let n = toast.actions.len();
            for (i, (name, run)) in toast.actions.iter().enumerate() {
                let run = run.clone();
                buttons = buttons.child(
                    crate::components::button(("toast-action", (id as usize) * 16 + i), name.clone(), &t)
                        .border_1()
                        .border_color(t.line)
                        .debug_selector(move || format!("toast-action-{id}-{i}"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close(id, cx);
                            run(window, cx);
                        })),
                );
            }
            for (j, action) in toast.notes.iter().enumerate() {
                let i = n + j;
                let action = action.clone();
                buttons = buttons.child(
                    crate::components::button(("toast-action", (id as usize) * 16 + i), action.label.clone(), &t)
                        .border_1()
                        .border_color(t.line)
                        .debug_selector(move || format!("toast-action-{id}-{i}"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close(id, cx);
                            if let Some(resolve) = this.resolve.clone() {
                                resolve(&action, window, cx);
                            }
                        })),
                );
            }
            let has_buttons = !toast.actions.is_empty() || !toast.notes.is_empty();
            let row = div()
                .id(("toast", id as usize))
                .debug_selector(move || format!("toast-{id}"))
                .flex()
                .items_start()
                .gap_2()
                .w(width)
                .pl_3()
                .pr_1()
                .py_1()
                .bg(t.raised)
                .border_1()
                .border_color(border)
                .rounded(t.radius_m)
                .occlude()
                .on_hover(cx.listener(move |this, on: &bool, _, cx| this.hover(id, *on, cx)))
                // The icon on the first line of text.
                .child(div().flex_none().h(t.line_height).flex().items_center().mt(t.size(2.)).child(icon(kind, color, &t)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .py(t.size(2.))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when_some(toast.title.clone(), |d, title| d.child(label(title).text_color(t.hi)))
                        .child(
                            // A click on the text opens the center on it: the whole text is there.
                            div()
                                .id(("toast-text", id as usize))
                                .debug_selector(move || format!("toast-text-{id}"))
                                .text_color(t.fg)
                                .whitespace_normal()
                                .line_clamp(TOAST_LINES)
                                .text_ellipsis()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| this.open(Some(id), window, cx)))
                                .child(toast.text.clone()),
                        )
                        .when(has_buttons, |d| d.child(buttons)),
                )
                .child(
                    crate::components::icon_button(("toast-close", id as usize), Icon::Close, "dismiss", &t)
                        .debug_selector(move || format!("toast-close-{id}"))
                        .on_click(cx.listener(move |this, _, _, cx| this.close(id, cx))),
                );
            stack = stack.child(row);
        }
        let waiting = self.queue.waiting();
        if waiting > 0 {
            stack = stack.child(
                crate::components::button("toasts-more", format!("+{waiting} more"), &t)
                    .debug_selector(|| "toasts-more".into())
                    .bg(t.raised)
                    .border_1()
                    .border_color(t.line)
                    .occlude()
                    .tooltip(crate::components::tip("notifications"))
                    .on_click(cx.listener(|this, _, window, cx| this.open(None, window, cx))),
            );
        }
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
        assert_eq!(step_selection(&items, Some(0), 1), Some(2), "disabled stays selectable");
        assert_eq!(step_selection(&items, Some(3), 1), Some(0), "wraps");
        assert_eq!(step_selection(&items, Some(0), -1), Some(3));
        assert_eq!(step_selection(&items, None, -1), Some(3));
        assert_eq!(step_selection(&[MenuItem::separator()], None, 1), None);
        assert_eq!(step_selection(&[], None, 1), None);
    }

    /// Lifetimes 8 s and 12 s for errors (were 6 and 10), and the queue keeps what it
    /// can't show instead of dropping it (it dropped past 4): changed on purpose with the
    /// notification center (plan "notifications", step A).
    #[test]
    fn toast_queue_expiry_shown_and_waiting() {
        let t0 = Instant::now();
        let mut q = ToastQueue::default();
        q.push(ToastKind::Info, "a".into(), vec![], t0);
        q.push(ToastKind::Err, "b".into(), vec![], t0);
        assert_eq!(q.expire(t0), Some(t0 + Duration::from_secs(8)));
        assert_eq!(q.expire(t0 + Duration::from_secs(9)), Some(t0 + Duration::from_secs(12)));
        assert_eq!(q.items.len(), 1, "info expired, error stays longer");
        assert_eq!(q.expire(t0 + Duration::from_secs(13)), None);
        for i in 0..6 {
            q.push(ToastKind::Info, format!("{i}").into(), vec![], t0);
        }
        assert_eq!(q.items.len(), 6, "nothing dropped");
        assert_eq!(q.shown().iter().map(|t| t.text.as_ref()).collect::<Vec<_>>(), ["0", "1", "2"]);
        assert_eq!(q.waiting(), 3);
        assert!(q.items[3..].iter().all(|t| t.deadline.is_none()), "waiting ones haven't started their lifetime");
        let t1 = t0 + Duration::from_secs(9);
        assert_eq!(q.expire(t1), Some(t1 + Duration::from_secs(8)), "the shown ones left, the waiting ones start now");
        assert_eq!(q.shown().iter().map(|t| t.text.as_ref()).collect::<Vec<_>>(), ["3", "4", "5"]);
        assert_eq!(q.waiting(), 0);
        let id = q.items[1].id;
        q.dismiss(id);
        assert_eq!(q.items.len(), 2);
    }

    #[test]
    fn the_live_queue_is_bounded_by_dropping_the_oldest_waiting() {
        let t0 = Instant::now();
        let mut q = ToastQueue::default();
        for i in 0..ToastQueue::LIVE + 2 {
            q.push(ToastKind::Info, format!("{i}").into(), vec![], t0);
        }
        assert_eq!(q.items.len(), ToastQueue::LIVE);
        assert_eq!(q.shown().iter().map(|t| t.text.as_ref()).collect::<Vec<_>>(), ["0", "1", "2"], "the shown ones stay");
        assert_eq!(q.items[ToastQueue::SHOWN].text.as_ref(), "5", "the oldest waiting went");
    }

    #[test]
    fn a_paused_toast_keeps_the_time_it_had_left() {
        let t0 = Instant::now();
        let mut q = ToastQueue::default();
        let a = q.push(ToastKind::Info, "a".into(), vec![], t0);
        q.pause(a, t0 + Duration::from_secs(5));
        assert_eq!(q.expire(t0 + Duration::from_secs(60)), None, "paused: no deadline, not expired");
        assert_eq!(q.items.len(), 1);
        let back = t0 + Duration::from_secs(60);
        q.resume(a, back);
        assert_eq!(q.expire(back), Some(back + Duration::from_secs(3)), "3 s were left");
        assert_eq!(q.expire(back + Duration::from_secs(4)), None);
        assert!(q.items.is_empty());
    }

    fn note(id: u64, kind: ToastKind) -> Note {
        Note { id, kind, title: None, text: format!("{id}").into(), at: Instant::now(), source: None, actions: vec![], read: false }
    }

    #[test]
    fn the_store_keeps_the_newest_500_and_counts_unread() {
        let mut s = NoteStore::default();
        for i in 0..(NoteStore::MAX as u64 + 5) {
            s.add(note(i, if i % 3 == 0 { ToastKind::Err } else if i % 3 == 1 { ToastKind::Warn } else { ToastKind::Info }));
        }
        assert_eq!(s.len(), NoteStore::MAX);
        assert_eq!(s.notes().front().map(|n| n.id), Some(NoteStore::MAX as u64 + 4), "newest first");
        assert_eq!(s.notes().back().map(|n| n.id), Some(5), "the oldest went");
        assert_eq!(s.unread(), NoteStore::MAX);
        let errs = s.notes().iter().filter(|n| n.kind == ToastKind::Err).count();
        assert_eq!(s.unread_errors(), errs, "warnings never count as errors");
        assert!(s.mark_read(6));
        assert!(!s.mark_read(6), "already read");
        assert_eq!(s.unread_errors(), errs - 1);
        assert_eq!(s.mark_read_where(|n| n.kind == ToastKind::Warn), s.notes().iter().filter(|n| n.kind == ToastKind::Warn).count());
        assert!(s.delete(7));
        assert!(!s.delete(7));
        assert_eq!(s.len(), NoteStore::MAX - 1);
        s.delete_where(|n| n.kind == ToastKind::Err);
        assert_eq!(s.unread_errors(), 0);
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn toast_history_keeps_the_last_ones_with_their_ids() {
        let t0 = Instant::now();
        let mut q = ToastQueue::default();
        let first = q.push(ToastKind::Warn, "first".into(), vec![], t0);
        assert_eq!(q.history.front(), Some(&Told { id: first, kind: ToastKind::Warn, text: "first".into() }));
        q.expire(t0 + Duration::from_secs(60));
        assert_eq!(q.history.len(), 1, "expired, still told");
        for i in 0..ToastQueue::HISTORY {
            q.push(ToastKind::Info, format!("{i}").into(), vec![], t0);
        }
        assert_eq!(q.history.len(), ToastQueue::HISTORY);
        assert_eq!(q.history.front().map(|t| t.text.as_ref()), Some("0"), "the oldest went");
        assert_eq!(q.history.back().map(|t| t.id), Some(first + ToastQueue::HISTORY as u64));
    }

    #[test]
    fn titles_follow_the_history_beside_it() {
        let t0 = Instant::now();
        let mut q = ToastQueue::default();
        let first = q.push_new(NewToast::new(ToastKind::Ok, "rolled out").title("svc on dev"), t0);
        assert_eq!(q.history.front(), Some(&Told { id: first, kind: ToastKind::Ok, text: "rolled out".into() }), "the history as it was");
        assert_eq!(q.titles.get(&first).map(|t| t.as_ref()), Some("svc on dev"));
        for i in 0..ToastQueue::HISTORY {
            q.push(ToastKind::Info, format!("{i}").into(), vec![], t0);
        }
        assert!(q.titles.is_empty(), "a title goes with its toast's history entry");
    }
}
