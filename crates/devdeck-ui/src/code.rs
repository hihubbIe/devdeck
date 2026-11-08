//! Code and notes (R12): a read-only code view and the note editor, both gpui-component's
//! `Editor` with tree-sitter highlighting, and the rendered Markdown view. gpui-component
//! stays inside this module (R9.3): views use [`Code`] and [`markdown_view`].
//!
//! [`init`] runs gpui-component's init (its input's own is private to it), drops its only
//! global key (the inspector's, debug builds) and copies our theme into its theme;
//! [`sync_theme`] copies it again whenever ours changes (theme or UI scale).
//!
//! What's ours around the editor: a wheel gesture keeps the axis it starts on (R9.6, the
//! editor adds both axes of every event), right-click opens our menu (R5.2), the cursor
//! doesn't blink (idle stays idle, R1.3), and inside gpui-component's text fields
//! ([`EDITOR_CONTEXT`]: the editors and their find field) only the keys they use as text
//! are taken from devdeck ([`editor_takes`]); the editor's `alt-shift-arrows` go back to
//! devdeck (moving tabs).

use crate::{MenuItem, Theme, open_menu, theme};
use gpui::{
    App, AppContext, ClipboardItem, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Keystroke, MouseButton, OngoingScroll,
    ParentElement, Pixels, Point, Render, ScrollDelta, ScrollEvent, SharedString, Styled, Subscription, Window, canvas, div,
};
use gpui_component::input::{Editor, EditorState, InputEvent};
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

/// Esc in the editor, when it doesn't use it itself (a view leaves the editor on it).
pub use gpui_component::input::Escape as EditorEscape;

/// The key context of gpui-component's text fields: both editors and their find field.
pub const EDITOR_CONTEXT: &str = "Input";

/// Its keys devdeck keeps (moving tabs, R5.1): the editor's own use of them (select to a
/// word, add a cursor) has other keys.
const GIVEN_BACK: &[&str] = &["alt-shift-left", "alt-shift-right", "alt-shift-up", "alt-shift-down"];

/// Bigger than this, a file shows as plain text: highlighting is most of the memory a
/// file costs (step 1 of docs/plans/files-and-notes.md: 10 MB took 567 MB highlighted,
/// 163 MB plain).
pub const HIGHLIGHT_MAX_BYTES: usize = 1_000_000;
/// A line this long or longer makes a file plain text (a minified bundle).
pub const HIGHLIGHT_MAX_LINE: usize = 5_000;

/// Sets gpui-component up for devdeck. Call once, before the first window.
pub fn init(cx: &mut App) {
    gpui_component::init(cx);
    // Its inspector's key is its only binding outside its components' contexts; the
    // editor's alt-shift-arrows are devdeck's.
    let given_back: Vec<Keystroke> = GIVEN_BACK.iter().filter_map(|k| Keystroke::parse(k).ok()).collect();
    let kept: Vec<gpui::KeyBinding> = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|b| !b.action().name().ends_with("ToggleInspector"))
        .filter(|b| match b.keystrokes() {
            [k] => !given_back.iter().any(|g| g.modifiers == k.inner().modifiers && g.key == k.inner().key),
            _ => true,
        })
        .cloned()
        .collect();
    cx.clear_key_bindings();
    cx.bind_keys(kept);
    gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
    if cx.has_global::<Theme>() {
        sync_theme(cx);
    }
}

/// Copies our theme (colors, font, text size) into gpui-component's.
pub fn sync_theme(cx: &mut App) {
    let t = theme(cx).clone();
    gpui_component::Theme::update(cx, |g| {
        g.font_family = t.font.clone();
        g.mono_font_family = t.font.clone();
        g.font_size = t.text;
        g.mono_font_size = t.text;
        // A blinking cursor redraws twice a second forever (R1.3), as our own inputs don't.
        g.cursor_blink = gpui_component::input::CursorBlink::steady();
        let c = &mut g.colors;
        c.background = t.bg;
        c.foreground = t.fg;
        c.border = t.line;
        c.input = t.line;
        c.muted = t.bar;
        c.muted_foreground = t.mute;
        c.selection = t.selection;
        c.caret = t.fg;
        c.accent = t.hover;
        c.accent_foreground = t.fg;
        c.link = t.accent;
        c.link_hover = t.accent;
        c.link_active = t.accent;
        c.popover = t.raised;
        c.popover_foreground = t.fg;
        c.ring = t.accent;
        c.scrollbar = t.bg;
        c.scrollbar_thumb = t.line;
        c.scrollbar_thumb_hover = t.dim;
        let mut h = (*gpui_component::highlighter::HighlightTheme::default_dark()).clone();
        h.style.editor_background = Some(t.bg);
        h.style.editor_foreground = Some(t.fg);
        h.style.editor_line_number = Some(t.dim);
        h.style.editor_active_line_number = Some(t.fg);
        h.style.editor_active_line = Some(t.bar);
        h.style.editor_gutter_background = Some(t.bg);
        g.highlight_theme = std::sync::Arc::new(h);
    });
}

/// The highlighting language of a file, by its name; `text` (none) when unknown.
pub fn language_for(path: &Path) -> &'static str {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_lowercase();
    if matches!(name.as_str(), "makefile" | "gnumakefile") || name.ends_with(".mk") {
        return "make";
    }
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "go" => "go",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" | "mdx" => "markdown",
        "sh" | "bash" | "zsh" => "bash",
        "css" | "scss" => "css",
        "html" | "htm" => "html",
        "py" | "pyi" => "python",
        "diff" | "patch" => "diff",
        "sql" => "sql",
        _ => "text",
    }
}

/// Whether `text` is small enough to highlight ([`HIGHLIGHT_MAX_BYTES`],
/// [`HIGHLIGHT_MAX_LINE`]).
pub fn highlightable(text: &str) -> bool {
    text.len() <= HIGHLIGHT_MAX_BYTES && text.lines().all(|l| l.len() < HIGHLIGHT_MAX_LINE)
}

/// True when a focused editor (or its find field) uses `key` (a binding's key, `ctrl-k`)
/// itself: text and moving keys (no modifier but shift), and the editing keys it binds.
/// Those of devdeck's bindings are off inside [`EDITOR_CONTEXT`]; the others (ctrl+w,
/// ctrl+tab, alt+arrows, alt+shift+arrows…) keep working there.
pub fn editor_takes(key: &'static str) -> bool {
    let Ok(k) = Keystroke::parse(key) else { return false };
    let m = k.modifiers;
    if !m.control && !m.alt && !m.platform && !m.function {
        return true;
    }
    EDITOR_KEYS.contains(&key)
}

/// The editor's own keys with ctrl or alt (gpui-component's `Input` bindings on Linux,
/// less [`GIVEN_BACK`]; `tests/code.rs` checks the list against them both ways).
pub const EDITOR_KEYS: &[&str] = &[
    "ctrl-a",
    "ctrl-c",
    "ctrl-x",
    "ctrl-v",
    "ctrl-z",
    "ctrl-y",
    "ctrl-f",
    "ctrl-h",
    "ctrl-enter",
    "ctrl-.",
    "ctrl-]",
    "ctrl-[",
    "ctrl-left",
    "ctrl-right",
    "ctrl-home",
    "ctrl-end",
    "ctrl-backspace",
    "ctrl-delete",
    "ctrl-shift-left",
    "ctrl-shift-right",
    "ctrl-shift-home",
    "ctrl-shift-end",
];

/// What a [`Code`] reports.
#[derive(Clone, Debug, PartialEq)]
pub enum CodeEvent {
    /// The user changed the text (note editor only).
    Changed,
}

/// Extra right-click items given the selected text (`None` without a selection).
pub type MenuFn = Rc<dyn Fn(Option<String>, &mut Window, &mut App) -> Vec<MenuItem>>;

/// A code view (read only) or a note editor, over gpui-component's editor.
pub struct Code {
    state: Entity<EditorState>,
    editable: bool,
    /// Set while devdeck itself changes the text, so only the user's edits report.
    quiet: Rc<Cell<bool>>,
    scroll: OngoingScroll,
    menu: Option<MenuFn>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<CodeEvent> for Code {}

impl Code {
    /// A read-only view of code: line numbers, find, no wrapping (R4.4), no folding.
    pub fn viewer(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| EditorState::new(window, cx).line_number(true).searchable(true).soft_wrap(false).folding(false).scroll_bounce(false));
        state.update(cx, |s, cx| s.set_readonly(true, cx));
        Self::new(state, false, cx)
    }

    /// The note editor: Markdown, wrapping (prose, a view's choice under R4.4), no line
    /// numbers.
    pub fn note(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| {
            EditorState::new(window, cx).language("markdown").line_number(false).searchable(true).soft_wrap(true).folding(false).scroll_bounce(false)
        });
        Self::new(state, true, cx)
    }

    fn new(state: Entity<EditorState>, editable: bool, cx: &mut Context<Self>) -> Self {
        let quiet = Rc::new(Cell::new(false));
        let q = quiet.clone();
        let subs = vec![cx.subscribe(&state, move |_, _, e: &InputEvent, cx| {
            if editable && !q.get() && matches!(e, InputEvent::Change) {
                cx.emit(CodeEvent::Changed);
            }
        })];
        Self { state, editable, quiet, scroll: OngoingScroll::default(), menu: None, _subs: subs }
    }

    /// Replaces the text and its language (`text` for none), keeping the scroll.
    /// Doesn't report [`CodeEvent::Changed`]: only the user's edits do.
    pub fn set_text(&mut self, text: &str, language: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.quiet.set(true);
        self.state.update(cx, |s, cx| {
            let offset = s.scroll_offset();
            s.set_highlighter(language.to_string(), cx);
            s.set_value(text.to_string(), window, cx);
            s.set_scroll_offset(offset, cx);
        });
        self.quiet.set(false);
    }

    /// The text shown.
    pub fn text(&self, cx: &App) -> SharedString {
        self.state.read(cx).value()
    }

    /// The selected text, if any.
    pub fn selected_text(&self, cx: &App) -> Option<String> {
        let s = self.state.read(cx).selected_text().to_string();
        (!s.is_empty()).then_some(s)
    }

    pub fn scroll_offset(&self, cx: &App) -> Point<Pixels> {
        self.state.read(cx).scroll_offset()
    }

    /// Scrolls to `offset` (clamped once laid out).
    pub fn set_scroll_offset(&mut self, offset: Point<Pixels>, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.set_scroll_offset(offset, cx));
    }

    /// Items the view adds to the right-click menu, after Copy (and the note's edits).
    pub fn set_menu(&mut self, menu: impl Fn(Option<String>, &mut Window, &mut App) -> Vec<MenuItem> + 'static) {
        self.menu = Some(Rc::new(menu));
    }

    /// Opens the right-click menu at `position`.
    pub fn open_menu(this: &Entity<Self>, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
        let c = this.read(cx);
        let (selected, editable, state, menu) = (c.selected_text(cx), c.editable, c.state.clone(), c.menu.clone());
        let mut items = Vec::new();
        if editable {
            items.push(dispatch("Cut", Box::new(gpui_component::input::Cut), &state));
        }
        let copy = selected.clone();
        let mut copy_item = MenuItem::new("Copy", move |_, cx| {
            if let Some(s) = &copy {
                cx.write_to_clipboard(ClipboardItem::new_string(s.clone()));
            }
        });
        if selected.is_none() {
            copy_item.disabled = Some("nothing selected".into());
        }
        items.push(copy_item);
        if editable {
            items.push(dispatch("Paste", Box::new(gpui_component::input::Paste), &state));
        }
        items.push(dispatch("Select all", Box::new(gpui_component::input::SelectAll), &state));
        if let Some(menu) = menu {
            let extra = menu(selected, window, cx);
            if !extra.is_empty() {
                items.push(MenuItem::separator());
                items.extend(extra);
            }
        }
        open_menu(position, items, window, cx);
    }

    /// Moves the editor by a wheel event, on one axis for a gesture (R9.6).
    fn scroll(&mut self, e: &ScrollEvent, cx: &mut Context<Self>) {
        let line = self.state.read(cx).line_height().unwrap_or(theme(cx).line_height);
        let mut delta = e.delta.pixel_delta(line);
        if matches!(e.delta, ScrollDelta::Pixels(_)) {
            self.scroll.filter(&mut delta, e.touch_phase);
        }
        let to = self.scroll_offset(cx) + delta;
        self.set_scroll_offset(to, cx);
    }
}

/// A menu item running `action` in the editor (focused first).
fn dispatch(label: &'static str, action: Box<dyn gpui::Action>, state: &Entity<EditorState>) -> MenuItem {
    let state = state.clone();
    MenuItem::new(label, move |window, cx| {
        state.update(cx, |s, cx| s.focus(window, cx));
        window.dispatch_action(action.boxed_clone(), cx);
    })
}

impl Focusable for Code {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.read(cx).focus_handle(cx)
    }
}

impl Render for Code {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let me = cx.entity().downgrade();
        let (wheel, menu) = (me.clone(), me);
        // Wheel and right-click are taken before the editor sees them (capture phase), when
        // nothing covers it (a menu, the palette, a modal).
        let catcher = canvas(
            |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                let wheel = wheel.clone();
                let over = hitbox.clone();
                window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Capture && over.should_handle_scroll(window) {
                        cx.stop_propagation();
                        let _ = wheel.update(cx, |c, cx| c.scroll(e, cx));
                    }
                });
                let menu = menu.clone();
                window.on_mouse_event(move |e: &gpui::MouseDownEvent, phase, window, cx| {
                    if phase == gpui::DispatchPhase::Capture && e.button == MouseButton::Right && hitbox.is_hovered(window) {
                        cx.stop_propagation();
                        window.prevent_default();
                        if let Some(c) = menu.upgrade() {
                            Code::open_menu(&c, e.position, window, cx);
                        }
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .relative()
            .size_full()
            .font_family(t.font.clone())
            .text_size(t.text)
            .child(Editor::new(&self.state).readonly(!self.editable).bordered(false).appearance(false).h_full())
            .child(catcher)
    }
}

/// `text` rendered as Markdown, in our font, wrapping (prose, R4.4).
pub fn markdown_view(id: impl Into<ElementId>, text: impl Into<SharedString>, t: &Theme) -> impl IntoElement {
    div().size_full().font_family(t.font.clone()).text_size(t.text).text_color(t.fg).child(gpui_component::text::TextView::markdown(id, text).selectable(true))
}
