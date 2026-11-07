//! A terminal's content (R6.7): bytes from a process go in, rows of styled text come out.
//! Wraps `alacritty_terminal` (the emulator Zed uses) so the UI never sees its types and
//! so a terminal works without any view: a run keeps its output while nothing shows it.
//!
//! Rows are numbered from the oldest history line (0) to the last used line, so a view
//! can list them like lines. Replies the emulator owes the process (cursor position,
//! device attributes) are collected for the caller to write back.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{self, NamedColor};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.cols as usize
    }
}

/// A color as the program asked for it; the theme gives it a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    /// The default text color.
    Fg,
    /// The default background (nothing drawn).
    Bg,
    /// One of the 16 ANSI colors (8–15 bright).
    Ansi(u8),
    /// Rgb, from the 256-color palette or given directly.
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub dim: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self { fg: Color::Fg, bg: Color::Bg, bold: false, italic: false, underline: false, strike: false, dim: false }
    }
}

/// `len` bytes of a row's text from `start`, in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
    pub style: Style,
}

/// One row: its text (trailing blanks dropped) and the styles over it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub text: String,
    pub spans: Vec<Span>,
}

/// Collects what the emulator must send back to the process.
#[derive(Clone, Default)]
struct Replies(Rc<RefCell<Vec<u8>>>);

impl EventListener for Replies {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event {
            self.0.borrow_mut().extend_from_slice(text.as_bytes());
        }
    }
}

pub struct Terminal {
    term: Term<Replies>,
    parser: ansi::Processor,
    replies: Replies,
    size: TermSize,
}

impl Terminal {
    /// An empty terminal of `size`, keeping `scrollback` lines of history.
    pub fn new(size: TermSize, scrollback: usize) -> Self {
        let size = TermSize { cols: size.cols.max(2), rows: size.rows.max(1) };
        let replies = Replies::default();
        let config = Config { scrolling_history: scrollback, ..Default::default() };
        Self { term: Term::new(config, &size, replies.clone()), parser: ansi::Processor::new(), replies, size }
    }

    pub fn size(&self) -> TermSize {
        self.size
    }

    /// Output of the process.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// What the process must receive back (answers to its queries), emptied.
    pub fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies.0.borrow_mut())
    }

    /// New size; lines rewrap to the new width.
    pub fn resize(&mut self, size: TermSize) {
        let size = TermSize { cols: size.cols.max(2), rows: size.rows.max(1) };
        if size != self.size {
            self.size = size;
            self.term.resize(size);
        }
    }

    fn history(&self) -> usize {
        self.term.grid().history_size()
    }

    /// Rows worth showing: the history, then the screen down to its last written line or
    /// the cursor, whichever is lower (a fresh screen isn't a block of blank rows).
    pub fn rows(&self) -> usize {
        let grid = self.term.grid();
        let screen = grid.screen_lines();
        let cursor = grid.cursor.point.line.0.max(0) as usize;
        let last_written = (0..screen).rev().find(|l| !row_is_blank(&grid[Line(*l as i32)])).unwrap_or(0);
        self.history() + cursor.max(last_written) + 1
    }

    /// Row `i`, 0 being the oldest history line.
    pub fn row(&self, i: usize) -> Row {
        let line = Line(i as i32 - self.history() as i32);
        if line < self.term.grid().topmost_line() || line > self.term.grid().bottommost_line() {
            return Row::default();
        }
        let cells = &self.term.grid()[line];
        let mut row = Row::default();
        // Up to the last cell holding something visible.
        let end = (0..self.term.grid().columns()).rev().find(|c| !is_blank(&cells[Column(*c)])).map_or(0, |c| c + 1);
        for c in 0..end {
            let cell = &cells[Column(c)];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let style = style_of(cell);
            let start = row.text.len();
            row.text.push(if cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c });
            if let Some(extra) = cell.zerowidth() {
                row.text.extend(extra);
            }
            let len = row.text.len() - start;
            match row.spans.last_mut() {
                Some(s) if s.style == style => s.len += len,
                _ => row.spans.push(Span { start, len, style }),
            }
        }
        row
    }

    /// True when the cursor is at the start of a line (output ended with a newline).
    pub fn at_line_start(&self) -> bool {
        self.term.grid().cursor.point.column.0 == 0
    }

    /// Every row's text (tests, find).
    pub fn lines(&self) -> Vec<String> {
        (0..self.rows()).map(|i| self.row(i).text).collect()
    }
}

fn is_blank(cell: &Cell) -> bool {
    (cell.c == ' ' || cell.c == '\0') && cell.bg == ansi::Color::Named(NamedColor::Background) && !cell.flags.intersects(Flags::INVERSE | Flags::ALL_UNDERLINES)
}

fn row_is_blank(row: &alacritty_terminal::grid::Row<Cell>) -> bool {
    (0..row.len()).all(|c| is_blank(&row[Column(c)]))
}

fn style_of(cell: &Cell) -> Style {
    let f = cell.flags;
    let bold = f.contains(Flags::BOLD);
    let mut fg = color(cell.fg, Color::Fg);
    // Bold ANSI colors are drawn bright, as terminals do.
    if bold && let Color::Ansi(n @ 0..=7) = fg {
        fg = Color::Ansi(n + 8);
    }
    let mut bg = color(cell.bg, Color::Bg);
    if f.contains(Flags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    let dim = f.contains(Flags::DIM) || matches!(cell.fg, ansi::Color::Named(n) if is_dim(n));
    Style {
        fg,
        bg,
        bold,
        italic: f.contains(Flags::ITALIC),
        underline: f.intersects(Flags::ALL_UNDERLINES),
        strike: f.contains(Flags::STRIKEOUT),
        dim,
    }
}

fn is_dim(n: NamedColor) -> bool {
    matches!(n as usize, 259..=266 | 268)
}

fn color(c: ansi::Color, default: Color) -> Color {
    match c {
        ansi::Color::Named(n) => match n as usize {
            i @ 0..=15 => Color::Ansi(i as u8),
            // Dim black..white.
            i @ 259..=266 => Color::Ansi((i - 259) as u8),
            _ => default,
        },
        ansi::Color::Indexed(i) if i < 16 => Color::Ansi(i),
        ansi::Color::Indexed(i) => {
            let (r, g, b) = palette(i);
            Color::Rgb(r, g, b)
        }
        ansi::Color::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

/// xterm's 256-color palette, entries 16–255: a 6×6×6 cube, then 24 grays.
pub fn palette(i: u8) -> (u8, u8, u8) {
    match i {
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
        _ => (0, 0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cols: u16, rows: u16) -> Terminal {
        Terminal::new(TermSize { cols, rows }, 100)
    }

    #[test]
    fn text_and_colors_come_out_as_rows() {
        let mut t = term(40, 5);
        t.feed(b"plain\r\n\x1b[31mERR\x1b[0m failed\r\n");
        assert_eq!(t.lines(), ["plain", "ERR failed", ""], "down to the cursor");
        let row = t.row(1);
        assert_eq!(row.spans[0], Span { start: 0, len: 3, style: Style { fg: Color::Ansi(1), ..Style::default() } });
        assert_eq!(row.spans[1].style, Style::default());
    }

    #[test]
    fn long_lines_wrap_at_the_width_and_rewrap_on_resize() {
        let mut t = term(10, 5);
        t.feed(b"0123456789abcdef");
        assert_eq!(t.lines(), ["0123456789", "abcdef"]);
        t.resize(TermSize { cols: 20, rows: 5 });
        assert_eq!(t.lines(), ["0123456789abcdef"], "reflowed");
    }

    #[test]
    fn history_keeps_lines_that_scrolled_off_the_screen() {
        let mut t = term(20, 3);
        for i in 0..10 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        let lines = t.lines();
        assert_eq!(lines.first().map(String::as_str), Some("line 0"));
        assert!(lines.contains(&"line 9".to_string()));
    }

    #[test]
    fn queries_get_answers_for_the_process() {
        let mut t = term(20, 5);
        t.feed(b"ab\x1b[6n");
        assert_eq!(t.take_replies(), b"\x1b[1;3R", "cursor position report");
        assert!(t.take_replies().is_empty());
    }

    #[test]
    fn bold_brightens_inverse_swaps_and_256_colors_resolve() {
        let mut t = term(20, 2);
        t.feed(b"\x1b[1;32mA\x1b[0;7mB\x1b[0;38;5;196mC");
        let s: Vec<Style> = t.row(0).spans.iter().map(|s| s.style).collect();
        assert_eq!((s[0].fg, s[0].bold), (Color::Ansi(10), true));
        assert_eq!((s[1].fg, s[1].bg), (Color::Bg, Color::Fg));
        assert_eq!(s[2].fg, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn a_clear_screen_leaves_no_blank_block() {
        let mut t = term(20, 10);
        t.feed(b"old\r\n\x1b[2J\x1b[Hnew");
        assert_eq!(t.lines().last().map(String::as_str), Some("new"));
        assert!(t.rows() <= 2, "{:?}", t.lines());
    }
}
