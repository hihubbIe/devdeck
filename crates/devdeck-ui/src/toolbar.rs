//! The toolbar every view draws with (R9.9): one line under a pane's tabs, its items in
//! the view's order, then "more". A custom element: in prepaint, with its real width, it
//! measures its items (labels shaped by the text system, paddings from the constants it
//! draws with), picks its form ([`fit`]), builds the row for that form, lays it out and
//! prepaints it, as `uniform_list` does its rows. No frame late, no redraw.
//!
//! - **Wide**: icons and labels, menu values, status words, counts.
//! - **Compact**: labels dropped (but `label_required` and `primary` ones), status words
//!   dropped, menu values kept.
//! - **Narrow**: items fold into "more" by [`Fold::Order`], 1 first, equal numbers
//!   together, until it fits. Never: [`Fold::Never`], `primary`, `state_shown`, statuses,
//!   fields, text.
//! - **Floor**: everything that folds is folded; the field gives up its minimum and
//!   truncates; at the extreme it becomes a button that focuses it (and shows while it has
//!   focus).
//!
//! Counts reserve their width, so the form depends on the width alone. No key is ever
//! shown (R5.4): tooltips name the item or say why it can't run (R5.5). Right-click on
//! the toolbar lists every item, shown or folded (R5.2).

use crate::components::tip;
use crate::icons::{Icon, icon};
use crate::overlay::{MenuItem, open_menu};
use crate::theme::{Theme, theme};
use gpui::{
    AnyElement, AnyView, App, AvailableSpace, Bounds, Element, ElementId, FocusHandle, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, MouseButton, Pixels, Point, SharedString, Stateful, Style, TextRun, Window, canvas, div, font, point,
    prelude::*, relative, size,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// A menu's items, built when it opens.
pub type MenuItems = Rc<dyn Fn(&mut App) -> Vec<MenuItem>>;
/// What an item does.
pub type ToolRun = Rc<dyn Fn(&mut Window, &mut App)>;

/// The id under which "more" publishes its bounds ([`ToolbarState::bounds`]).
pub const MORE: &str = "more";

// Sizes the toolbar draws with and measures with (logical pixels, before the UI scale).
/// Between items.
const GAP: f32 = 4.;
/// Inside a labeled item, each side.
const PAD: f32 = 6.;
/// Between an item's icon, text and count.
const INNER: f32 = 4.;
/// Before the first item.
const EDGE_START: f32 = 6.;
/// After the last item: apart from the pane header's icons above.
const EDGE_END: f32 = 12.;
/// A text item asks for this many characters before it truncates.
const TEXT_CELLS: usize = 10;
/// A field squeezed at the floor keeps this many characters before it becomes a button.
const FIELD_FLOOR_CELLS: usize = 3;

// ------------------------------------------------------------------ model

/// When an item folds into "more" in the narrow form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Never,
    /// 1 folds first; equal numbers fold together.
    Order(u8),
}

/// A count after an item or a field ("3/12", "40 new"). It reserves `cells` characters
/// whatever its text, so the form doesn't change with it; the view caps its text ("999+").
#[derive(Clone)]
pub struct Count {
    pub text: SharedString,
    pub cells: usize,
    pub color: Hsla,
}

impl Count {
    pub fn new(text: impl Into<SharedString>, cells: usize, color: Hsla) -> Self {
        Self { text: text.into(), cells, color }
    }
}

/// An action or toggle: the registered action's id, label and run (the app builds it
/// from its registry).
#[derive(Clone)]
pub struct ToolAction {
    pub id: SharedString,
    pub icon: Option<Icon>,
    /// Shown wide; the tooltip and the menu text otherwise.
    pub label: SharedString,
    /// Some: a toggle, framed while on.
    pub on: Option<bool>,
    /// Not in its default state (a filter set, paused): never folds.
    pub state_shown: bool,
    /// Never icon-only: keeps its label in compact, folds instead.
    pub label_required: bool,
    /// Never folds, keeps its label.
    pub primary: bool,
    /// Can't run now, and why (R5.5): dim, still clickable (its run explains).
    pub why_not: Option<SharedString>,
    pub fold: Fold,
    pub count: Option<Count>,
    pub run: ToolRun,
}

impl ToolAction {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, run: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            on: None,
            state_shown: false,
            label_required: false,
            primary: false,
            why_not: None,
            fold: Fold::Never,
            count: None,
            run: Rc::new(run),
        }
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn toggle(mut self, on: bool) -> Self {
        self.on = Some(on);
        self
    }
    pub fn state_shown(mut self, shown: bool) -> Self {
        self.state_shown = shown;
        self
    }
    pub fn label_required(mut self) -> Self {
        self.label_required = true;
        self
    }
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }
    pub fn disabled(mut self, reason: impl Into<SharedString>) -> Self {
        self.why_not = Some(reason.into());
        self
    }
    /// [`ToolAction::disabled`] when there is a reason.
    pub fn why_not(mut self, reason: Option<SharedString>) -> Self {
        self.why_not = reason;
        self
    }
    pub fn fold(mut self, fold: Fold) -> Self {
        self.fold = fold;
        self
    }
    pub fn count(mut self, count: Option<Count>) -> Self {
        self.count = count;
        self
    }

    fn folds(&self) -> Option<u8> {
        match self.fold {
            Fold::Order(n) if !self.primary && !self.state_shown => Some(n),
            _ => None,
        }
    }

    /// The tooltip: why it can't run, or its name. Never a key (R5.4).
    fn tip(&self) -> SharedString {
        self.why_not.clone().unwrap_or_else(|| self.label.clone())
    }

    /// Its text in a menu, its count after it.
    fn menu_text(&self) -> SharedString {
        match &self.count {
            Some(c) if !c.text.is_empty() => format!("{} · {}", self.label, c.text).into(),
            _ => self.label.clone(),
        }
    }

    /// Its row in a menu: a toggle checked when on (the menu stays open when it's used), a
    /// disabled one with its reason.
    fn menu_item(&self, text: SharedString) -> MenuItem {
        let run = self.run.clone();
        let item = MenuItem::new(text, move |w, cx| run(w, cx));
        let mut item = match self.on {
            Some(on) => item.toggle(on),
            None => item,
        };
        item.disabled = self.why_not.clone();
        item
    }
}

/// What a menu button shows: its value ("warn+", an env), maybe in a color with an icon.
#[derive(Clone)]
pub struct MenuValue {
    pub text: SharedString,
    pub color: Option<Hsla>,
    pub icon: Option<Icon>,
}

impl MenuValue {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into(), color: None, icon: None }
    }
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// One item of a toolbar.
#[derive(Clone)]
pub enum ToolItem {
    /// A text field (a [`crate::TextInput`]): grows, at least `min_chars` wide until the
    /// floor; its count at its right end, its accessories after it. `focus` is the input's
    /// focus handle (the floor's button and the right-click menu focus it); `label` names
    /// it there.
    Field { id: SharedString, label: SharedString, input: AnyView, focus: FocusHandle, min_chars: usize, count: Option<Count>, accessories: Vec<ToolAction> },
    Action(ToolAction),
    /// A button opening a menu under it; its value shown (kept in compact), capped at
    /// `max_chars` (0: no cap). Folded, a submenu (its items have no submenus).
    Menu { action: ToolAction, value: Option<MenuValue>, max_chars: usize, items: MenuItems },
    /// A state in one icon (a word when wide); a click opens its details.
    Status { id: SharedString, icon: Icon, color: Hsla, word: Option<SharedString>, tip: SharedString, details: MenuItems },
    /// Flexible text: truncates, full text on hover (R4.4).
    Text { text: SharedString, color: Hsla },
    /// What follows is right-aligned.
    Spacer,
}

impl ToolItem {
    fn id(&self) -> Option<&SharedString> {
        match self {
            ToolItem::Field { id, .. } | ToolItem::Status { id, .. } => Some(id),
            ToolItem::Action(a) | ToolItem::Menu { action: a, .. } => Some(&a.id),
            ToolItem::Text { .. } | ToolItem::Spacer => None,
        }
    }

    /// Its count, folded: "more" shows a dot in its color.
    fn count(&self) -> Option<&Count> {
        match self {
            ToolItem::Action(a) | ToolItem::Menu { action: a, .. } => a.count.as_ref(),
            _ => None,
        }
    }

    /// Its rows in a menu ("more", right-click): an action, a toggle checked, a menu as a
    /// submenu, a status with its details, a field (focuses it) and its accessories.
    fn menu_items(&self, cx: &mut App) -> Vec<MenuItem> {
        match self {
            ToolItem::Action(a) => vec![a.menu_item(a.menu_text())],
            ToolItem::Menu { action, value, items, .. } => {
                let sub = items(cx);
                assert!(sub.iter().all(|i| i.submenu.is_empty()), "{}: a toolbar menu's items have no submenus (menus are one level deep)", action.id);
                let text = match value {
                    Some(v) => format!("{}: {}", action.menu_text(), v.text).into(),
                    None => action.menu_text(),
                };
                vec![action.menu_item(text).submenu(sub)]
            }
            ToolItem::Status { word, tip, details, id, .. } => {
                let sub = details(cx);
                assert!(sub.iter().all(|i| i.submenu.is_empty()), "{id}: a status's details have no submenus");
                let mut item = MenuItem::new(word.clone().unwrap_or_else(|| tip.clone()), |_, _| {}).submenu(sub);
                item.run = None;
                vec![item]
            }
            ToolItem::Field { label, focus, accessories, .. } => {
                let focus = focus.clone();
                let mut rows = vec![MenuItem::new(label.clone(), move |w, cx| w.focus(&focus, cx))];
                rows.extend(accessories.iter().map(|a| a.menu_item(a.menu_text())));
                rows
            }
            ToolItem::Text { .. } | ToolItem::Spacer => Vec::new(),
        }
    }
}

// ------------------------------------------------------------------ fit (pure)

/// One item's widths, for [`fit`].
#[derive(Clone, Debug, PartialEq)]
pub struct FitItem {
    pub wide: Pixels,
    pub compact: Pixels,
    /// Folds in the narrow form, in this order; None: never.
    pub fold: Option<u8>,
    /// Flexible: the width it can shrink to at the floor.
    pub squeeze: Option<Pixels>,
    /// At the extreme it becomes a button this wide (a field).
    pub button: Option<Pixels>,
    /// A field with focus never becomes a button.
    pub focused: bool,
}

impl FitItem {
    /// An item as wide in every form, never folding.
    pub fn fixed(w: Pixels) -> Self {
        Self { wide: w, compact: w, fold: None, squeeze: None, button: None, focused: false }
    }
}

/// What [`fit`] chooses from.
#[derive(Clone, Debug, PartialEq)]
pub struct FitInput {
    pub items: Vec<FitItem>,
    /// Between two items.
    pub gap: Pixels,
    /// Both ends' insets together.
    pub edges: Pixels,
    /// The "more" button's width.
    pub more: Pixels,
    /// The view put items in "more": it shows in every form.
    pub more_items: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Wide,
    Compact,
    Narrow,
    Floor,
}

/// How one item shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Wide,
    Compact,
    Folded,
    /// At the floor: shrinks below its minimum.
    Squeezed,
    /// At the extreme: a field shown as a button.
    Button,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub shape: Shape,
    pub items: Vec<Shown>,
    pub more: bool,
}

/// The width a row needs with each item shown as said.
fn need(input: &FitInput, shown: &[Shown], more: bool) -> Pixels {
    let mut w = input.edges;
    let mut n = 0usize;
    for (it, s) in input.items.iter().zip(shown) {
        w += match s {
            Shown::Wide => it.wide,
            Shown::Compact => it.compact,
            Shown::Folded => continue,
            Shown::Squeezed => it.squeeze.unwrap_or(it.compact),
            Shown::Button => it.button.unwrap_or(it.compact),
        };
        n += 1;
    }
    if more {
        w += input.more;
        n += 1;
    }
    w + input.gap * n.saturating_sub(1) as f32
}

/// The form for `available` pixels: wide if all fits, else compact, else narrow (folding
/// groups by order, 1 first), else the floor.
pub fn fit(input: &FitInput, available: Pixels) -> Form {
    let n = input.items.len();
    let fits = |shown: &[Shown]| {
        let more = input.more_items || shown.contains(&Shown::Folded);
        (need(input, shown, more) <= available).then_some(more)
    };
    let done = |shape, items: Vec<Shown>, more| Form { shape, items, more };
    let wide = vec![Shown::Wide; n];
    if let Some(more) = fits(&wide) {
        return done(Shape::Wide, wide, more);
    }
    let mut cur = vec![Shown::Compact; n];
    if let Some(more) = fits(&cur) {
        return done(Shape::Compact, cur, more);
    }
    let mut orders: Vec<u8> = input.items.iter().filter_map(|i| i.fold).collect();
    orders.sort_unstable();
    orders.dedup();
    for order in orders {
        for (i, it) in input.items.iter().enumerate() {
            if it.fold == Some(order) {
                cur[i] = Shown::Folded;
            }
        }
        if let Some(more) = fits(&cur) {
            return done(Shape::Narrow, cur, more);
        }
    }
    for (i, it) in input.items.iter().enumerate() {
        if it.squeeze.is_some() {
            cur[i] = Shown::Squeezed;
        }
    }
    if fits(&cur).is_none() {
        for (i, it) in input.items.iter().enumerate() {
            if it.button.is_some() && !it.focused {
                cur[i] = Shown::Button;
            }
        }
    }
    let more = input.more_items || cur.contains(&Shown::Folded);
    done(Shape::Floor, cur, more)
}

// ------------------------------------------------------------------ state

/// What the toolbar learnt at its last layout, for the view that owns it: each item's
/// bounds (window coordinates) and which ones folded, so menus opened by key open under
/// their button, or under "more" when folded.
#[derive(Default)]
pub struct ToolbarState {
    bounds: RefCell<HashMap<SharedString, Bounds<Pixels>>>,
    row: Cell<Option<Bounds<Pixels>>>,
    folded: RefCell<Vec<SharedString>>,
    shape: Cell<Option<Shape>>,
}

impl ToolbarState {
    pub fn new() -> Rc<Self> {
        Rc::default()
    }

    /// Where item `id` (or [`MORE`]) was drawn last; None when folded or not shown.
    pub fn bounds(&self, id: &str) -> Option<Bounds<Pixels>> {
        self.bounds.borrow().get(id).copied()
    }

    /// The ids folded into "more" at the last layout, in the toolbar's order.
    pub fn folded(&self) -> Vec<SharedString> {
        self.folded.borrow().clone()
    }

    pub fn is_folded(&self, id: &str) -> bool {
        self.folded.borrow().iter().any(|f| f == id)
    }

    /// The form of the last layout.
    pub fn shape(&self) -> Option<Shape> {
        self.shape.get()
    }

    /// Where a menu of item `id` opens: under it, under "more" when folded, else under the
    /// toolbar's right end.
    fn anchor(&self, id: &str) -> Option<Point<Pixels>> {
        let b = if self.is_folded(id) { self.bounds(MORE) } else { self.bounds(id) };
        b.map(|b| b.bottom_left()).or_else(|| self.row.get().map(|r| point(r.right() - r.size.height * 4., r.bottom())))
    }

    fn record(&self, id: SharedString, b: Bounds<Pixels>) {
        self.bounds.borrow_mut().insert(id, b);
    }
}

/// Opens a menu under item `id` (see [`ToolbarState::anchor`]), after the current event:
/// the items may read the view that handles it.
fn open_under(state: &Rc<ToolbarState>, id: &str, items: impl FnOnce(&mut App) -> Vec<MenuItem> + 'static, window: &mut Window, cx: &mut App) {
    let Some(at) = state.anchor(id) else { return };
    open_at(at, items, window, cx);
}

fn open_at(at: Point<Pixels>, items: impl FnOnce(&mut App) -> Vec<MenuItem> + 'static, window: &mut Window, cx: &mut App) {
    window.defer(cx, move |window, cx| {
        let items = items(cx);
        if !items.is_empty() {
            open_menu(at, items, window, cx);
        }
    });
}

/// "More"'s menu: its figures (a disabled first row), the folded items in order, then the
/// view's own items.
fn more_menu(folded: Vec<ToolItem>, figures: Option<SharedString>, more: Option<MenuItems>) -> impl FnOnce(&mut App) -> Vec<MenuItem> + 'static {
    move |cx| {
        let mut out = Vec::new();
        if let Some(f) = figures {
            let mut row = MenuItem::new(f, |_, _| {}).disabled("");
            row.run = None;
            out.push(row);
        }
        let folded: Vec<MenuItem> = folded.iter().flat_map(|i| i.menu_items(cx)).collect();
        let own = more.map(|m| m(cx)).unwrap_or_default();
        let split = !folded.is_empty() && !own.is_empty();
        out.extend(folded);
        if split {
            out.push(MenuItem::separator());
        }
        out.extend(own);
        out
    }
}

// ------------------------------------------------------------------ the element

/// A view's toolbar. Build it in `render` with a [`ToolbarState`] the view keeps; build it
/// again to open its menus by key ([`Toolbar::open_more`], [`Toolbar::open_menu_of`]).
pub struct Toolbar {
    id: SharedString,
    items: Vec<ToolItem>,
    more: Option<MenuItems>,
    figures: Option<SharedString>,
    state: Rc<ToolbarState>,
}

impl Toolbar {
    pub fn new(id: impl Into<SharedString>, state: Rc<ToolbarState>) -> Self {
        Self { id: id.into(), items: Vec::new(), more: None, figures: None, state }
    }

    pub fn item(mut self, item: ToolItem) -> Self {
        self.items.push(item);
        self
    }

    /// What the view always puts in "more", and its figures ("3 pods · 1200 lines"): the
    /// first row of "more" and its tooltip.
    pub fn more(mut self, items: MenuItems, figures: Option<SharedString>) -> Self {
        self.more = Some(items);
        self.figures = figures;
        self
    }

    /// Opens "more" under its button (by key).
    pub fn open_more(&self, window: &mut Window, cx: &mut App) {
        let folded = self.folded_items();
        open_under(&self.state, MORE, more_menu(folded, self.figures.clone(), self.more.clone()), window, cx);
    }

    /// Opens the menu of item `id` (a menu button's items, a status's details) under its
    /// button, or under "more" when folded (by key). An action runs.
    pub fn open_menu_of(&self, id: &str, window: &mut Window, cx: &mut App) {
        let Some(item) = self.items.iter().find(|i| i.id().is_some_and(|i| i == id)) else { return };
        match item {
            ToolItem::Menu { items, .. } | ToolItem::Status { details: items, .. } => {
                let items = items.clone();
                open_under(&self.state, id, move |cx| items(cx), window, cx);
            }
            ToolItem::Action(a) => (a.run)(window, cx),
            ToolItem::Field { focus, .. } => window.focus(focus, cx),
            ToolItem::Text { .. } | ToolItem::Spacer => {}
        }
    }

    fn folded_items(&self) -> Vec<ToolItem> {
        self.items.iter().filter(|i| i.id().is_some_and(|id| self.state.is_folded(id))).cloned().collect()
    }
}

/// The toolbar's sizes for this frame.
struct Metrics {
    gap: Pixels,
    pad: Pixels,
    inner: Pixels,
    edge_start: Pixels,
    edge_end: Pixels,
    square: Pixels,
    icon: Pixels,
    hair: Pixels,
    cell: Pixels,
    /// A text input's padding and border, both sides (`TextInput`: `px_2`, `border_1`).
    input_chrome: Pixels,
}

impl Metrics {
    fn new(window: &Window, t: &Theme) -> Self {
        Self {
            gap: t.size(GAP),
            pad: t.size(PAD),
            inner: t.size(INNER),
            edge_start: t.size(EDGE_START),
            edge_end: t.size(EDGE_END),
            square: t.button_h,
            icon: t.icon,
            hair: t.hairline,
            cell: crate::terminal::cell_size(window, t).width,
            input_chrome: window.rem_size() + t.hairline * 2.,
        }
    }

    fn cells(&self, n: usize) -> Pixels {
        (self.cell * n as f32).ceil()
    }

    /// A framed item holding `parts` (icons, text, count) with its padding.
    fn frame(&self, parts: &[Pixels]) -> Pixels {
        let sum = parts.iter().fold(Pixels::ZERO, |a, b| a + *b);
        (self.pad + self.hair) * 2. + sum + self.inner * parts.len().saturating_sub(1) as f32
    }
}

fn text_w(window: &Window, t: &Theme, s: &SharedString) -> Pixels {
    if s.is_empty() {
        return Pixels::ZERO;
    }
    let run = TextRun { len: s.len(), font: font(t.font.clone()), color: t.fg, background_color: None, underline: None, strikethrough: None };
    window.text_system().shape_line(s.clone(), t.text, &[run], None).width.ceil()
}

/// What an item measured to, for fitting and for drawing it.
struct Plan {
    fit: FitItem,
    /// Its count's reserved width.
    count: Pixels,
}

fn plan(item: &ToolItem, m: &Metrics, window: &Window, t: &Theme, cx: &App) -> Plan {
    let count_w = |c: &Option<Count>| c.as_ref().map(|c| m.cells(c.cells));
    let parts = |icons: usize, text: Option<Pixels>, count: Option<Pixels>| {
        if icons == 1 && text.is_none() && count.is_none() {
            return m.square;
        }
        let mut p = vec![m.icon; icons];
        p.extend(text);
        p.extend(count);
        m.frame(&p)
    };
    match item {
        ToolItem::Action(a) => {
            let text = text_w(window, t, &a.label);
            let count = count_w(&a.count);
            let icons = a.icon.is_some() as usize;
            let wide = parts(icons, Some(text), count);
            let keeps_label = a.label_required || a.primary || a.icon.is_none();
            let compact = if keeps_label { wide } else { parts(icons, None, count) };
            Plan { fit: FitItem { wide, compact, fold: a.folds(), squeeze: None, button: None, focused: false }, count: count.unwrap_or_default() }
        }
        ToolItem::Menu { action: a, value, max_chars, .. } => {
            let count = count_w(&a.count);
            let icons = a.icon.is_some() as usize;
            let (wide, compact) = match value {
                Some(v) => {
                    let mut text = text_w(window, t, &v.text);
                    if *max_chars > 0 {
                        text = text.min(m.cells(*max_chars));
                    }
                    let w = parts(icons + v.icon.is_some() as usize, Some(text), count);
                    (w, w)
                }
                None => {
                    let text = text_w(window, t, &a.label);
                    let wide = parts(icons, Some(text), count);
                    let keeps_label = a.label_required || a.primary || a.icon.is_none();
                    (wide, if keeps_label { wide } else { parts(icons, None, count) })
                }
            };
            Plan { fit: FitItem { wide, compact, fold: a.folds(), squeeze: None, button: None, focused: false }, count: count.unwrap_or_default() }
        }
        ToolItem::Status { word, .. } => {
            let text = word.as_ref().map_or(Pixels::ZERO, |w| text_w(window, t, w));
            let wide = if word.is_some() { parts(1, Some(text), None) } else { m.square };
            Plan { fit: FitItem { wide, compact: m.square, fold: None, squeeze: None, button: None, focused: false }, count: Pixels::ZERO }
        }
        ToolItem::Field { focus, min_chars, count, accessories, .. } => {
            let count = count_w(count);
            let extras = count.map_or(Pixels::ZERO, |c| m.inner + c) + (m.inner + m.square) * accessories.len() as f32;
            let min = m.cells(*min_chars) + m.input_chrome + extras;
            let squeeze = m.cells(FIELD_FLOOR_CELLS) + m.input_chrome + extras;
            let fit = FitItem { wide: min, compact: min, fold: None, squeeze: Some(squeeze.min(min)), button: Some(m.square), focused: focus.contains_focused(window, cx) };
            Plan { fit, count: count.unwrap_or_default() }
        }
        ToolItem::Text { text, .. } => {
            let w = text_w(window, t, text);
            let min = w.min(m.cells(TEXT_CELLS));
            Plan { fit: FitItem { wide: min, compact: min, fold: None, squeeze: Some(Pixels::ZERO), button: None, focused: false }, count: Pixels::ZERO }
        }
        ToolItem::Spacer => Plan { fit: FitItem::fixed(Pixels::ZERO), count: Pixels::ZERO },
    }
}

/// The hover group of an item: its icon brightens with it.
const GROUP: &str = "tool";

/// An item's frame: `button_h` tall, `w` wide, one toggle look (fill and frame on,
/// nothing off), dim when it can't run. It publishes its bounds to the state.
fn frame(key: SharedString, w: Pixels, on: bool, enabled: bool, tooltip: SharedString, state: &Rc<ToolbarState>, t: &Theme) -> Stateful<gpui::Div> {
    let selector = format!("tool-{key}");
    let (hover, fg) = (t.hover, t.fg);
    let st = state.clone();
    let k = key.clone();
    div()
        .id(ElementId::Name(format!("tool-{key}").into()))
        .debug_selector(move || selector.clone())
        .group(GROUP)
        .relative()
        .flex_none()
        .w(w)
        .h(t.button_h)
        .flex()
        .items_center()
        .gap(t.size(INNER))
        .rounded(t.radius_m)
        .border_1()
        .border_color(if on { t.accent } else { gpui::transparent_black() })
        .when(on, |d| d.bg(t.accent_bg))
        .text_color(if !enabled { t.dim } else if on { t.hi } else { t.mute })
        .cursor_pointer()
        .whitespace_nowrap()
        .when(enabled, |d| d.hover(move |s| s.bg(hover).text_color(fg)))
        .tooltip(tip(tooltip))
        // A toolbar in a draggable title bar mustn't start a window move.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(canvas(move |b, _, _| st.record(k.clone(), b), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
}

/// An item's icon: muted (bright with the item under the pointer), dim when it can't run,
/// or in its own color.
fn item_icon(i: Icon, enabled: bool, color: Option<Hsla>, t: &Theme) -> gpui::Svg {
    let fg = t.fg;
    match (enabled, color) {
        (false, _) => icon(i, t.dim, t),
        (true, Some(c)) => icon(i, c, t),
        (true, None) => icon(i, t.mute, t).group_hover(GROUP, move |s| s.text_color(fg)),
    }
}

fn count_el(c: &Count, w: Pixels) -> gpui::Div {
    div().flex_none().w(w).overflow_hidden().text_color(c.color).child(c.text.clone())
}

fn text_el(text: SharedString) -> gpui::Div {
    div().flex_1().min_w_0().truncate().child(text)
}

/// Draws one item as `shown`; None when folded.
fn draw(item: &ToolItem, index: usize, shown: Shown, p: &Plan, m: &Metrics, state: &Rc<ToolbarState>, t: &Theme) -> Option<AnyElement> {
    if shown == Shown::Folded {
        return None;
    }
    let pad = |d: Stateful<gpui::Div>, w: Pixels| if w == m.square { d.justify_center() } else { d.px(m.pad) };
    let w = if shown == Shown::Wide { p.fit.wide } else { p.fit.compact };
    Some(match item {
        ToolItem::Action(a) => {
            let enabled = a.why_not.is_none();
            let labeled = shown == Shown::Wide || a.label_required || a.primary || a.icon.is_none();
            let run = a.run.clone();
            let f = frame(a.id.clone(), w, a.on == Some(true), enabled, a.tip(), state, t);
            pad(f, w)
                .when(a.primary && enabled, |d| d.text_color(t.accent))
                .when_some(a.icon, |d, i| d.child(item_icon(i, enabled, None, t)))
                .when(labeled, |d| d.child(text_el(a.label.clone())))
                .when_some(a.count.as_ref(), |d, c| d.child(count_el(c, p.count)))
                .on_click(move |_, window, cx| run(window, cx))
                .into_any_element()
        }
        ToolItem::Menu { action: a, value, items, .. } => {
            let enabled = a.why_not.is_none();
            let tooltip = match (&a.why_not, value) {
                (Some(why), _) => why.clone(),
                (None, Some(v)) => format!("{}: {}", a.label, v.text).into(),
                (None, None) => a.label.clone(),
            };
            let labeled = value.is_some() || shown == Shown::Wide || a.label_required || a.primary || a.icon.is_none();
            let (run, items, st, id) = (a.run.clone(), items.clone(), state.clone(), a.id.clone());
            let f = frame(a.id.clone(), w, a.on == Some(true), enabled, tooltip, state, t);
            pad(f, w)
                .when_some(a.icon, |d, i| d.child(item_icon(i, enabled, None, t)))
                .when_some(value.as_ref().and_then(|v| v.icon.map(|i| (i, v.color))), |d, (i, c)| d.child(item_icon(i, enabled, c, t)))
                .when(labeled, |d| match value {
                    Some(v) => d.child(text_el(v.text.clone()).when_some(v.color.filter(|_| enabled), |d, c| d.text_color(c))),
                    None => d.child(text_el(a.label.clone())),
                })
                .when_some(a.count.as_ref(), |d, c| d.child(count_el(c, p.count)))
                .on_click(move |_, window, cx| {
                    if enabled {
                        let items = items.clone();
                        open_under(&st, &id, move |cx| items(cx), window, cx);
                    } else {
                        run(window, cx);
                    }
                })
                .into_any_element()
        }
        ToolItem::Status { id, icon: i, color, word, tip, details } => {
            let (st, sid, details) = (state.clone(), id.clone(), details.clone());
            let f = frame(id.clone(), w, false, true, tip.clone(), state, t);
            pad(f, w)
                .child(item_icon(*i, true, Some(*color), t))
                .when_some(word.clone().filter(|_| shown == Shown::Wide), |d, word| d.child(text_el(word).text_color(*color)))
                .on_click(move |_, window, cx| {
                    let details = details.clone();
                    open_under(&st, &sid, move |cx| details(cx), window, cx);
                })
                .into_any_element()
        }
        ToolItem::Field { id, label, input, focus, count, accessories, .. } => {
            if shown == Shown::Button {
                let focus = focus.clone();
                return Some(
                    pad(frame(id.clone(), m.square, false, true, label.clone(), state, t), m.square)
                        .child(item_icon(Icon::Search, true, None, t))
                        .on_click(move |_, window, cx| {
                            window.focus(&focus, cx);
                            window.refresh();
                        })
                        .into_any_element(),
                );
            }
            let min = if shown == Shown::Squeezed { p.fit.squeeze.unwrap_or(p.fit.compact) } else { p.fit.compact };
            let selector = format!("tool-{id}");
            let (st, key) = (state.clone(), id.clone());
            div()
                .debug_selector(move || selector.clone())
                .relative()
                .flex_1()
                .min_w(min)
                .h(t.button_h)
                .flex()
                .items_center()
                .gap(m.inner)
                .child(div().flex_1().min_w_0().child(input.clone()))
                .when_some(count.as_ref(), |d, c| d.child(count_el(c, p.count)))
                .children(accessories.iter().map(|a| {
                    let enabled = a.why_not.is_none();
                    let run = a.run.clone();
                    frame(a.id.clone(), m.square, a.on == Some(true), enabled, a.tip(), state, t)
                        .justify_center()
                        .child(item_icon(a.icon.unwrap_or(Icon::Plus), enabled, None, t))
                        .on_click(move |_, window, cx| run(window, cx))
                        .into_any_element()
                }))
                .child(canvas(move |b, _, _| st.record(key.clone(), b), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
                .into_any_element()
        }
        ToolItem::Text { text, color } => div()
            .id(("tool-text", index))
            .debug_selector(move || format!("tool-text-{index}"))
            .min_w_0()
            .truncate()
            .text_color(*color)
            .tooltip(tip(text.clone()))
            .child(text.clone())
            .into_any_element(),
        ToolItem::Spacer => div().flex_1().into_any_element(),
    })
}

/// "More": an ellipsis, a dot in a folded count's color, its tooltip "more" and the
/// figures.
fn draw_more(dot: Option<Hsla>, open: impl Fn(&mut Window, &mut App) + 'static, figures: &Option<SharedString>, m: &Metrics, state: &Rc<ToolbarState>, t: &Theme) -> AnyElement {
    let tooltip: SharedString = match figures {
        Some(f) => format!("more · {f}").into(),
        None => "more".into(),
    };
    let side = t.size(6.);
    frame(MORE.into(), m.square, false, true, tooltip, state, t)
        .justify_center()
        .child(item_icon(Icon::More, true, None, t))
        .when_some(dot, |d, c| {
            d.child(div().debug_selector(|| "tool-more-dot".into()).absolute().top(t.size(2.)).right(t.size(2.)).size(side).rounded_full().bg(c))
        })
        .on_click(move |_, window, cx| open(window, cx))
        .into_any_element()
}

impl IntoElement for Toolbar {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Toolbar {
    type RequestLayoutState = ();
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::Name(self.id.clone()))
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    /// One full-width leaf, the toolbar's height: its row is laid out in prepaint.
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let t = theme(cx);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = t.toolbar_h.into();
        style.flex_shrink = 0.;
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let t = theme(cx).clone();
        let m = Metrics::new(window, &t);
        let plans: Vec<Plan> = self.items.iter().map(|i| plan(i, &m, window, &t, cx)).collect();
        let more_items = self.more.as_ref().is_some_and(|f| !f(cx).is_empty());
        let input = FitInput { items: plans.iter().map(|p| p.fit.clone()).collect(), gap: m.gap, edges: m.edge_start + m.edge_end, more: m.square, more_items };
        let form = fit(&input, bounds.size.width);

        let state = self.state.clone();
        state.bounds.borrow_mut().clear();
        state.row.set(Some(bounds));
        state.shape.set(Some(form.shape));
        let folded: Vec<ToolItem> = self.items.iter().zip(&form.items).filter(|(_, s)| **s == Shown::Folded).map(|(i, _)| i.clone()).collect();
        *state.folded.borrow_mut() = folded.iter().filter_map(|i| i.id().cloned()).collect();

        let mut row = div()
            .size_full()
            .flex()
            .items_center()
            .gap(m.gap)
            .pl(m.edge_start)
            .pr(m.edge_end)
            .overflow_hidden()
            .border_b_1()
            .border_color(t.line_soft)
            .font_family(t.font.clone())
            .text_size(t.text)
            .whitespace_nowrap();
        for (i, (item, shown)) in self.items.iter().zip(&form.items).enumerate() {
            if let Some(el) = draw(item, i, *shown, &plans[i], &m, &state, &t) {
                row = row.child(el);
            }
        }
        if form.more {
            let dot = folded.iter().find_map(|i| i.count()).map(|c| c.color);
            let (st, figures, more) = (state.clone(), self.figures.clone(), self.more.clone());
            let folded_more = folded.clone();
            let open = move |window: &mut Window, cx: &mut App| open_under(&st, MORE, more_menu(folded_more.clone(), figures.clone(), more.clone()), window, cx);
            row = row.child(draw_more(dot, open, &self.figures, &m, &state, &t));
        }
        // Right-click: every item, shown and folded, in order (R5.2).
        let all = self.items.clone();
        let mut row = div()
            .id(ElementId::Name(format!("{}-row", self.id).into()))
            .size_full()
            .child(row)
            .on_mouse_down(MouseButton::Right, move |e, window, cx| {
                cx.stop_propagation();
                let all = all.clone();
                open_at(e.position, move |cx| all.iter().flat_map(|i| i.menu_items(cx)).collect(), window, cx);
            })
            .into_any_element();
        row.layout_as_root(size(AvailableSpace::Definite(bounds.size.width), AvailableSpace::Definite(bounds.size.height)), window, cx);
        row.prepaint_at(bounds.origin, window, cx);
        Some(row)
    }

    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), row: &mut Option<AnyElement>, window: &mut Window, cx: &mut App) {
        if let Some(row) = row {
            row.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    /// An action `w` wide, `c` compact, folding at `fold`.
    fn act(w: f32, c: f32, fold: Option<u8>) -> FitItem {
        FitItem { wide: px(w), compact: px(c), fold, squeeze: None, button: None, focused: false }
    }

    fn field(min: f32, squeeze: f32) -> FitItem {
        FitItem { wide: px(min), compact: px(min), fold: None, squeeze: Some(px(squeeze)), button: Some(px(24.)), focused: false }
    }

    /// No gaps or edges, "more" 20 wide: widths add up plainly.
    fn input(items: Vec<FitItem>, more_items: bool) -> FitInput {
        FitInput { items, gap: px(0.), edges: px(0.), more: px(20.), more_items }
    }

    #[test]
    fn wide_then_compact_at_their_thresholds() {
        let i = input(vec![field(100., 30.), act(60., 30., Some(1)), act(50., 20., None)], false);
        assert_eq!(fit(&i, px(210.)).shape, Shape::Wide, "exactly fits");
        assert_eq!(fit(&i, px(209.)).shape, Shape::Compact);
        assert_eq!(fit(&i, px(150.)).shape, Shape::Compact, "100 + 30 + 20");
        let f = fit(&i, px(149.));
        assert_eq!((f.shape, f.items[1], f.more), (Shape::Narrow, Shown::Folded, true), "folds, \"more\" shows");
    }

    #[test]
    fn gaps_and_edges_count() {
        let i = FitInput { items: vec![act(10., 10., None), act(10., 10., None)], gap: px(4.), edges: px(6.), more: px(20.), more_items: false };
        assert_eq!(fit(&i, px(30.)).shape, Shape::Wide);
        assert_eq!(fit(&i, px(29.)).shape, Shape::Floor, "nothing folds");
        let with_more = FitInput { more_items: true, ..i };
        assert_eq!(fit(&with_more, px(54.)).shape, Shape::Wide, "the view's \"more\" and its gap");
        assert!(fit(&with_more, px(54.)).more);
    }

    #[test]
    fn folds_in_order_one_first_and_ties_together() {
        let i = input(vec![field(100., 30.), act(20., 20., Some(2)), act(20., 20., Some(1)), act(20., 20., Some(1)), act(20., 20., Some(3))], false);
        // Compact: 180. Folding 1s (both): 140 + more 20 = 160.
        let f = fit(&i, px(160.));
        assert_eq!(f.shape, Shape::Narrow);
        assert_eq!(f.items[1..], [Shown::Compact, Shown::Folded, Shown::Folded, Shown::Compact], "equal numbers fold together");
        let f = fit(&i, px(159.));
        assert_eq!(f.items[1..], [Shown::Folded, Shown::Folded, Shown::Folded, Shown::Compact], "then 2");
        let f = fit(&i, px(120.));
        assert_eq!(f.items[1..], [Shown::Folded; 4]);
        assert_eq!(f.shape, Shape::Narrow);
    }

    #[test]
    fn never_folding_kinds_stay() {
        // Never, primary, state shown and statuses all come in with `fold: None`.
        let i = input(vec![field(100., 30.), act(20., 20., None), act(20., 20., Some(1))], false);
        let f = fit(&i, px(10.));
        assert_eq!(f.items[1], Shown::Compact, "a non-folding action stays even at the floor");
        assert_eq!(f.items[2], Shown::Folded);
        assert_eq!(f.shape, Shape::Floor);
    }

    #[test]
    fn what_never_folds_by_its_flags() {
        let a = ToolAction::new("a", "a", |_, _| {}).fold(Fold::Order(1));
        assert_eq!(a.folds(), Some(1));
        assert_eq!(a.clone().primary().folds(), None, "primary");
        assert_eq!(a.clone().state_shown(true).folds(), None, "a state other than its default");
        assert_eq!(a.clone().fold(Fold::Never).folds(), None);
    }

    #[test]
    fn a_label_required_item_folds_instead_of_going_icon_only() {
        // Its compact width is its wide width: it can't drop the label, so it folds.
        let i = input(vec![field(100., 30.), act(60., 60., Some(1)), act(40., 20., Some(2))], false);
        let f = fit(&i, px(180.));
        assert_eq!(f.shape, Shape::Compact);
        assert_eq!(f.items[1], Shown::Compact, "compact keeps its label (60)");
        let f = fit(&i, px(179.));
        assert_eq!((f.shape, f.items[1], f.items[2]), (Shape::Narrow, Shown::Folded, Shown::Compact));
    }

    #[test]
    fn reserved_counts_dont_change_the_form() {
        // A count's width is reserved in the item's widths whatever its text: the same
        // input gives the same form, whatever the count says.
        let i = input(vec![field(100., 30.), act(60., 40., Some(1))], false);
        for w in [100., 139., 140., 159., 160., 300.] {
            assert_eq!(fit(&i, px(w)), fit(&i.clone(), px(w)));
        }
    }

    #[test]
    fn the_floor_squeezes_the_field_then_makes_it_a_button() {
        let i = input(vec![field(100., 30.), act(40., 30., Some(1)), act(40., 20., None)], false);
        // Compact: 150. All folded: 100 + 20 + more 20 = 140.
        assert_eq!(fit(&i, px(140.)).shape, Shape::Narrow);
        let f = fit(&i, px(139.));
        assert_eq!((f.shape, f.items[0]), (Shape::Floor, Shown::Squeezed), "the field gives up its minimum");
        let f = fit(&i, px(70.));
        assert_eq!(f.items[0], Shown::Squeezed, "30 + 20 + 20");
        let f = fit(&i, px(69.));
        assert_eq!(f.items[0], Shown::Button, "the extreme");
        assert!(f.more);
        let mut focused = i.clone();
        focused.items[0].focused = true;
        assert_eq!(fit(&focused, px(69.)).items[0], Shown::Squeezed, "with focus it stays a field");
    }

    #[test]
    fn more_shows_only_when_it_holds_something() {
        let i = input(vec![act(20., 20., Some(1))], false);
        assert!(!fit(&i, px(100.)).more);
        assert!(fit(&i, px(10.)).more, "folded");
        assert!(fit(&input(vec![act(20., 20., Some(1))], true), px(100.)).more, "the view's items");
    }

    #[test]
    fn tooltips_and_menu_rows_never_show_a_key() {
        let a = ToolAction::new("logs.wrap", "wrap long lines", |_, _| {}).toggle(true).count(Some(Count::new("3 new", 6, gpui::black())));
        assert_eq!(a.tip(), "wrap long lines");
        let item = a.menu_item(a.menu_text());
        assert_eq!((item.label.as_ref(), item.checked, item.hint.is_none()), ("wrap long lines · 3 new", true, true));
        let d = ToolAction::new("x", "x", |_, _| {}).disabled("no cluster");
        assert_eq!(d.tip(), "no cluster", "a disabled item's tooltip is its reason");
        assert_eq!(d.menu_item(d.menu_text()).disabled.as_deref(), Some("no cluster"));
    }
}
