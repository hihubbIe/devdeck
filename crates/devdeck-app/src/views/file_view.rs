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
use devdeck_ui::{Icon, Scrolls, label, markdown_view, theme};
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
    file: Entity<OpenFile>,
    /// The editor, made on the first draw (it needs a window).
    pub code: Option<Entity<Code>>,
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
            file,
            code: None,
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
        code
    }
}

impl Render for FileView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx).clone();
        let code = matches!(self.shown, Shown::Text { .. }).then(|| self.editor(window, cx));
        // Focus given to the view goes on to the text while it shows (copy, find).
        if let Some(c) = &code
            && !(self.markdown && self.rendered)
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
            .on_action(cx.listener(|this, _: &ToggleRendered, window, cx| this.toggle_rendered(window, cx)));
        match &self.shown {
            Shown::Loading => root.into_any_element(),
            Shown::Problem(why) => root
                .items_center()
                .justify_center()
                .text_color(t.mute)
                .debug_selector(|| "file-problem".into())
                .child(label(why.clone()))
                .into_any_element(),
            Shown::Text { .. } if self.markdown && self.rendered => root
                .child(
                    div()
                        .id("rendered")
                        .debug_selector(|| "file-rendered".into())
                        .flex_1()
                        .min_h_0()
                        .scroll_y()
                        .px_3()
                        .py_2()
                        .child(markdown_view("file-md", self.text.clone(), &t)),
                )
                .into_any_element(),
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
