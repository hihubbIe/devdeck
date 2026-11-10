//! File view (`file/<path>`, R12.1, R12.4): a text file, view only, highlighted by its
//! language; a Markdown file shows its source or rendered. It follows the file while
//! shown: a change on disk reads it again (at most once a second, keeping the scroll),
//! and one made while hidden is read when it comes back on screen. Reading happens off
//! the UI thread.
//!
//! Too big, not text, missing or unreadable: a message says so in place of the text, and
//! the tab stays (a file that comes back shows again). Over 1 MB or with a very long
//! line, the text shows plain, and a line above it says why.

use super::{KeepsState, Live, ToggleRendered, ViewAction, ViewCx};
use crate::open_file::OpenFile;
use devdeck_core::files::{FileText, read_text};
use devdeck_ui::code::{self, Code};
use devdeck_ui::{Icon, Rendered, Scrolls, label, theme};
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Render, SharedString, Styled,
    Subscription, Task, Window, div, point, prelude::FluentBuilder,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A file changing often (a growing log) is read again at most this often (R1.3).
pub const RELOAD_EVERY: Duration = Duration::from_secs(1);

/// What the view shows now.
#[derive(Clone, Debug, PartialEq)]
pub enum Shown {
    Loading,
    /// The text; `plain` when it's too big or wide to highlight.
    Text { plain: bool },
    /// Why there's no text.
    Problem(String),
}

pub struct FileView {
    pub path: PathBuf,
    ctx: ViewCx,
    file: Entity<OpenFile>,
    /// The editor, made on the first draw (it needs a window).
    pub code: Option<Entity<Code>>,
    /// The rendered Markdown, made when first shown.
    pub rendered_view: Option<Entity<Rendered>>,
    /// Text to put in the editor at the next draw: its language, and a scroll to restore.
    pending: Option<(SharedString, &'static str, Option<Point<Pixels>>)>,
    /// The text when it's shown (for the rendered view).
    text: SharedString,
    pub shown: Shown,
    /// Markdown: rendered rather than source (R12.4).
    pub rendered: bool,
    markdown: bool,
    focus: FocusHandle,
    /// The file's generation last read (or being read).
    read: Option<u64>,
    visible: bool,
    last_read: Option<Instant>,
    loading: Option<Task<()>>,
    /// The scroll to put back once the text is there (a restart).
    restore_scroll: Option<Point<Pixels>>,
    /// The line to go to once the editor has the text.
    go_to: Option<usize>,
    _subs: Vec<Subscription>,
}

impl FileView {
    pub fn new(path: PathBuf, ctx: ViewCx, cx: &mut Context<Self>) -> Self {
        let file = ctx.services.update(cx, |s, cx| s.file(&path, cx));
        let markdown = code::language_for(&path) == "markdown";
        let focus = cx.focus_handle();
        let mut subs = vec![cx.observe(&file, |this, _, cx| this.follow(cx))];
        subs.extend(ctx.on_visibility(cx, |this, shown, cx| {
            this.visible = shown;
            if shown {
                this.follow(cx);
            }
        }));
        let mut view = Self {
            path,
            ctx,
            file,
            code: None,
            rendered_view: None,
            pending: None,
            text: SharedString::default(),
            shown: Shown::Loading,
            rendered: markdown,
            markdown,
            focus,
            read: None,
            visible: false,
            last_read: None,
            loading: None,
            restore_scroll: None,
            go_to: None,
            _subs: subs,
        };
        view.load(cx);
        view
    }

    /// Reads the file again if it changed since the last read and the view is shown, no
    /// sooner than [`RELOAD_EVERY`] after the last read.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let generation = self.file.read(cx).generation;
        if self.read == Some(generation) || !self.visible || self.loading.is_some() {
            return;
        }
        let now = cx.background_executor().now();
        let wait = self.last_read.map_or(Duration::ZERO, |t| RELOAD_EVERY.saturating_sub(now.saturating_duration_since(t)));
        if wait.is_zero() {
            self.load(cx);
            return;
        }
        self.loading = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = None;
                this.follow(cx);
            });
        }));
    }

    /// Reads the file in the background, then shows it.
    fn load(&mut self, cx: &mut Context<Self>) {
        let generation = self.file.read(cx).generation;
        self.read = Some(generation);
        self.last_read = Some(cx.background_executor().now());
        let path = self.path.clone();
        let read = cx.background_spawn(async move { read_text(&path) });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let text = read.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = None;
                this.show(text, cx);
                // A change that came while reading.
                this.follow(cx);
            });
        }));
    }

    fn show(&mut self, text: FileText, cx: &mut Context<Self>) {
        match text {
            FileText::Text(text) => {
                let plain = !code::highlightable(&text);
                let language = if plain { "text" } else { code::language_for(&self.path) };
                self.shown = Shown::Text { plain };
                self.text = text.into();
                // The editor takes it at the next draw (it needs the window).
                self.pending = Some((self.text.clone(), language, self.restore_scroll.take()));
            }
            other => {
                self.shown = Shown::Problem(other.problem(&self.path).unwrap_or_default());
                self.text = SharedString::default();
            }
        }
        cx.notify();
    }

    /// Shows a Markdown file rendered or as its source, without moving the focus (the
    /// control API); false when it isn't Markdown.
    pub fn set_rendered(&mut self, rendered: bool, cx: &mut Context<Self>) -> bool {
        if !self.markdown {
            return false;
        }
        if self.rendered != rendered {
            self.rendered = rendered;
            cx.notify();
        }
        true
    }

    pub fn toggle_rendered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.markdown {
            return;
        }
        self.rendered = !self.rendered;
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Why the text shows plain, if it does.
    fn plain_reason(&self) -> Option<String> {
        if !matches!(self.shown, Shown::Text { plain: true }) {
            return None;
        }
        Some(if self.text.len() > code::HIGHLIGHT_MAX_BYTES {
            format!("Shown as plain text: over {} MB.", code::HIGHLIGHT_MAX_BYTES / 1_000_000)
        } else {
            format!("Shown as plain text: a line of {} characters or more.", code::HIGHLIGHT_MAX_LINE)
        })
    }
}

impl Live for FileView {
    fn actions(&self, _cx: &App) -> Vec<ViewAction> {
        if self.markdown { vec![ViewAction { id: TOGGLE_RENDERED, icon: Icon::Rendered, on: self.rendered }] } else { Vec::new() }
    }

    fn run(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if id == TOGGLE_RENDERED {
            self.toggle_rendered(window, cx);
        }
    }
}

/// The registry's id of [`ToggleRendered`].
pub const TOGGLE_RENDERED: &str = "view.toggle_rendered";

impl KeepsState for FileView {
    fn save(&self, cx: &App) -> serde_json::Value {
        let s = self.code.as_ref().map(|c| c.read(cx).scroll_offset(cx)).or(self.restore_scroll).unwrap_or_default();
        serde_json::json!({ "rendered": self.rendered, "scroll_x": f32::from(s.x), "scroll_y": f32::from(s.y) })
    }

    fn restore(&mut self, state: &serde_json::Value, cx: &mut Context<Self>) {
        if let Some(r) = state.get("rendered").and_then(serde_json::Value::as_bool) {
            self.rendered = r && self.markdown;
        }
        let f = |k: &str| state.get(k).and_then(serde_json::Value::as_f64).map(|v| devdeck_ui::logical(v as f32));
        if let (Some(x), Some(y)) = (f("scroll_x"), f("scroll_y")) {
            self.restore_scroll = Some(point(x, y));
        }
        cx.notify();
    }
}

impl Focusable for FileView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FileView {
    /// The editor, made now if it isn't yet, with the text that waits for it. Focus on
    /// the view goes on to it while it shows (copy and find work there).
    fn editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<Code> {
        let code = match &self.code {
            Some(c) => c.clone(),
            None => {
                let c = cx.new(|cx| Code::viewer(window, cx));
                // Right-click and the menu key: Copy, Share with… for the selection (R5.7).
                let menu = crate::context_menu::text_menu(&self.ctx);
                c.update(cx, |c, _| c.set_menu(menu));
                self.code = Some(c.clone());
                c
            }
        };
        if let Some((text, language, scroll)) = self.pending.take() {
            code.update(cx, |c, cx| {
                c.set_text(&text, language, window, cx);
                if let Some(s) = scroll {
                    c.set_scroll_offset(s, cx);
                }
            });
        }
        if matches!(self.shown, Shown::Text { .. })
            && let Some(line) = self.go_to.take()
        {
            code.update(cx, |c, cx| c.go_to_line(line, cx));
        }
        code
    }

    /// Shows line `line` (1 for the first), the caret at its start, once the source shows
    /// (the text read, the editor drawn); never moves the focus (R13.1).
    pub fn go_to_line(&mut self, line: usize, cx: &mut Context<Self>) {
        self.go_to = Some(line);
        cx.notify();
    }

    /// The rendered Markdown, made now if it isn't yet, showing the text.
    fn rendered_view(&mut self, cx: &mut Context<Self>) -> Entity<Rendered> {
        let text = self.text.clone();
        match &self.rendered_view {
            Some(r) => {
                r.update(cx, |r, cx| r.set_text(&text, cx));
                r.clone()
            }
            None => {
                let menu = crate::context_menu::text_menu(&self.ctx);
                let r = cx.new(|cx| {
                    let mut r = Rendered::new(&text, cx);
                    r.set_menu(menu);
                    r
                });
                self.rendered_view = Some(r.clone());
                r
            }
        }
    }

    /// The menu key (R5.7): the menu for the selection, else the key goes on (the tab's
    /// menu).
    fn menu_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let opened = match (&self.code, &self.rendered_view) {
            (_, Some(r)) if self.markdown && self.rendered => Rendered::menu_key(r, window, cx),
            (Some(c), _) if !(self.markdown && self.rendered) => Code::menu_key(c, window, cx),
            _ => false,
        };
        if !opened {
            cx.propagate();
        }
    }
}

impl Render for FileView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        // The editor only while the source shows: rendered Markdown doesn't pay for
        // highlighting it (the text waits in `pending`).
        let source = matches!(self.shown, Shown::Text { .. }) && !(self.markdown && self.rendered);
        let code = source.then(|| self.editor(window, cx));
        // Focus given to the view goes on to the text while it shows (copy, find).
        if let Some(c) = &code
            && self.focus.is_focused(window)
        {
            let f = c.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| f.focus(window, cx));
        }
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .key_context("FileView")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleRendered, window, cx| this.toggle_rendered(window, cx)))
            .on_action(cx.listener(|this, _: &crate::actions::ContextMenu, window, cx| this.menu_key(window, cx)));
        match &self.shown {
            Shown::Loading => root.into_any_element(),
            Shown::Problem(why) => root
                .justify_center()
                .px_3()
                .text_color(t.mute)
                .debug_selector(|| "file-problem".into())
                .child(div().w_full().min_w_0().flex().justify_center().child(label(why.clone()).min_w_0()))
                .into_any_element(),
            Shown::Text { .. } if self.markdown && self.rendered => {
                let rendered = self.rendered_view(cx);
                root.child(
                    div()
                        .id("rendered")
                        .debug_selector(|| "file-rendered".into())
                        .flex_1()
                        .min_h_0()
                        .scroll_y()
                        .px_3()
                        .py_2()
                        .child(rendered),
                )
                .into_any_element()
            }
            Shown::Text { .. } => root
                .when_some(self.plain_reason(), |d, why| {
                    d.child(
                        div()
                            .flex_none()
                            .h(t.row_h)
                            .px_2()
                            .flex()
                            .items_center()
                            .border_b_1()
                            .border_color(t.line)
                            .text_color(t.mute)
                            .debug_selector(|| "file-plain".into())
                            .child(label(why)),
                    )
                })
                .children(code.map(|c| div().flex_1().min_h_0().child(c)))
                .into_any_element(),
        }
    }
}
