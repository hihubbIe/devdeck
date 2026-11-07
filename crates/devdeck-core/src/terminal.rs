//! A terminal's content (R6.7): bytes from a process go in, rows of styled text come out.
//! Wraps `alacritty_terminal` (the emulator Zed uses) so the UI never sees its types and
//! so a terminal works without any view: a run keeps its output while nothing shows it.
//!
//! Rows are numbered from the oldest history line (0) to the last used line, so a view
//! can list them like lines. Replies the emulator owes the process (cursor position,
//! device attributes, colours, the text area's size) are collected for the caller to
//! write back. Keys and pastes become bytes for the process here too ([`key_bytes`],
//! [`paste_bytes`]), so the rules are tested without a UI.

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, NamedColor, Rgb};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

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

/// An `(r, g, b)` color.
pub type Rgb8 = (u8, u8, u8);

/// What terminals answer colour and size queries with: the theme's colors and the size of
/// a cell in pixels. The theme and the UI scale are the app's, so one look serves every
/// terminal; the UI sets it with [`set_look`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    /// The 16 ANSI colors (8–15 bright).
    pub ansi: [Rgb8; 16],
    pub fg: Rgb8,
    pub bg: Rgb8,
    pub cursor: Rgb8,
    /// Width and height of a cell, in pixels.
    pub cell: (u16, u16),
}

impl Look {
    /// xterm's colors on a dark background, until the UI sets its own.
    pub const DEFAULT: Look = Look {
        ansi: [
            (0, 0, 0),
            (205, 0, 0),
            (0, 205, 0),
            (205, 205, 0),
            (0, 0, 238),
            (205, 0, 205),
            (0, 205, 205),
            (229, 229, 229),
            (127, 127, 127),
            (255, 0, 0),
            (0, 255, 0),
            (255, 255, 0),
            (92, 92, 255),
            (255, 0, 255),
            (0, 255, 255),
            (255, 255, 255),
        ],
        fg: (229, 229, 229),
        bg: (0, 0, 0),
        cursor: (229, 229, 229),
        cell: (8, 16),
    };

    /// Color `i` of alacritty's table: 0–255 indexed, then foreground, background,
    /// cursor, the dim colors, bright and dim foreground.
    fn color(&self, i: usize) -> Rgb8 {
        let dim = |(r, g, b): Rgb8| ((r as u16 * 2 / 3) as u8, (g as u16 * 2 / 3) as u8, (b as u16 * 2 / 3) as u8);
        match i {
            0..=15 => self.ansi[i],
            16..=255 => palette(i as u8),
            257 => self.bg,
            258 => self.cursor,
            259..=266 => dim(self.ansi[i - 259]),
            268 => dim(self.fg),
            _ => self.fg,
        }
    }
}

thread_local! {
    static LOOK: std::cell::Cell<Look> = const { std::cell::Cell::new(Look::DEFAULT) };
}

/// The look every terminal of this thread answers queries with from now on.
pub fn set_look(look: Look) {
    LOOK.set(look);
}

pub fn look() -> Look {
    LOOK.get()
}

/// Terminal modes a view acts on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// Cursor keys send `ESC O A` rather than `ESC [ A` (DECCKM).
    pub app_cursor: bool,
    /// Pastes are wrapped in markers.
    pub bracketed_paste: bool,
    /// A full-screen program's own screen, without history.
    pub alt_screen: bool,
    /// On the alternate screen, the wheel sends cursor keys.
    pub alternate_scroll: bool,
    pub show_cursor: bool,
}

/// Where the cursor is: its row (numbered as [`Terminal::row`]), its column, and the byte
/// of the row's text under it (`None` past the end of the text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub byte: Option<usize>,
}

/// A reply the emulator owes the process. Colour and size replies are worded by the
/// emulator and filled in with the look when taken.
enum Reply {
    Text(String),
    Color(usize, Arc<dyn Fn(Rgb) -> String + Send + Sync>),
    Size(Arc<dyn Fn(WindowSize) -> String + Send + Sync>),
}

/// Collects what the emulator must send back to the process.
#[derive(Clone, Default)]
struct Replies(Rc<RefCell<Vec<Reply>>>);

impl EventListener for Replies {
    fn send_event(&self, event: Event) {
        let reply = match event {
            Event::PtyWrite(text) => Reply::Text(text),
            Event::ColorRequest(i, format) => Reply::Color(i, format),
            Event::TextAreaSizeRequest(format) => Reply::Size(format),
            _ => return,
        };
        self.0.borrow_mut().push(reply);
    }
}

pub struct Terminal {
    term: Term<Replies>,
    parser: ansi::Processor,
    replies: Replies,
    size: TermSize,
    /// Rows that fell off the top of the history so far.
    dropped: u64,
}

impl Terminal {
    /// An empty terminal of `size`, keeping `scrollback` lines of history.
    pub fn new(size: TermSize, scrollback: usize) -> Self {
        let size = TermSize { cols: size.cols.max(2), rows: size.rows.max(1) };
        let replies = Replies::default();
        let config = Config { scrolling_history: scrollback, ..Default::default() };
        Self { term: Term::new(config, &size, replies.clone()), parser: ansi::Processor::new(), replies, size, dropped: 0 }
    }

    pub fn size(&self) -> TermSize {
        self.size
    }

    /// Output of the process.
    pub fn feed(&mut self, bytes: &[u8]) {
        // Counts the rows that scroll into the history, to know how many fell off its top:
        // alacritty keeps a scrolled-back display on its rows by moving it up as rows
        // arrive, so a display one row up measures them. Exact unless one chunk scrolls
        // more rows than the history holds.
        let (alt, before) = (self.modes().alt_screen, self.history());
        let mut offset = self.term.grid().display_offset();
        if !alt && offset == 0 && before > 0 {
            self.term.scroll_display(Scroll::Delta(1));
            offset = 1;
        }
        self.parser.advance(&mut self.term, bytes);
        if !alt && !self.modes().alt_screen {
            let scrolled = self.term.grid().display_offset().saturating_sub(offset);
            self.dropped += (before + scrolled).saturating_sub(self.history()) as u64;
        }
        if self.term.grid().display_offset() != 0 {
            self.term.scroll_display(Scroll::Bottom);
        }
    }

    /// Rows that fell off the top of the history since the start: row `i` is row
    /// `i + dropped()` counted from the first row ever written, so a view keeps its place
    /// and its selection while output arrives.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// What the process must receive back (answers to its queries), emptied.
    pub fn take_replies(&mut self) -> Vec<u8> {
        let replies = std::mem::take(&mut *self.replies.0.borrow_mut());
        let look = look();
        let mut out = Vec::new();
        for reply in replies {
            let text = match reply {
                Reply::Text(text) => text,
                Reply::Color(i, format) => {
                    // A color the program set wins over the theme's.
                    let set = (i < alacritty_terminal::term::color::COUNT).then(|| self.term.colors()[i]).flatten();
                    format(set.unwrap_or_else(|| {
                        let (r, g, b) = look.color(i);
                        Rgb { r, g, b }
                    }))
                }
                Reply::Size(format) => format(WindowSize {
                    num_lines: self.size.rows,
                    num_cols: self.size.cols,
                    cell_width: look.cell.0,
                    cell_height: look.cell.1,
                }),
            };
            out.extend_from_slice(text.as_bytes());
        }
        out
    }

    pub fn modes(&self) -> Modes {
        let m = *self.term.mode();
        Modes {
            app_cursor: m.contains(TermMode::APP_CURSOR),
            bracketed_paste: m.contains(TermMode::BRACKETED_PASTE),
            alt_screen: m.contains(TermMode::ALT_SCREEN),
            alternate_scroll: m.contains(TermMode::ALTERNATE_SCROLL),
            show_cursor: m.contains(TermMode::SHOW_CURSOR),
        }
    }

    pub fn cursor(&self) -> Cursor {
        let point = self.term.grid().cursor.point;
        let row = self.history() + point.line.0.max(0) as usize;
        let col = point.column.0;
        let mut byte = None;
        if let Some(cells) = self.line(row) {
            let mut at = 0;
            for c in 0..self.end_of(cells) {
                let cell = &cells[Column(c)];
                if is_spacer(cell) {
                    continue;
                }
                if c >= col {
                    byte = (c == col).then_some(at);
                    break;
                }
                at += cell.c.len_utf8() + cell.zerowidth().map_or(0, |z| z.iter().map(|c| c.len_utf8()).sum());
            }
        }
        Cursor { row, col, byte }
    }

    /// The text from cell `from` to cell `to` (row, column), both included, in reading
    /// order: a row that wrapped runs on into the next, other rows end at their last
    /// visible character.
    pub fn text(&self, from: (usize, usize), to: (usize, usize)) -> String {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        let cols = self.term.grid().columns();
        let mut out = String::new();
        for i in from.0..=to.0 {
            let Some(cells) = self.line(i) else { break };
            let start = if i == from.0 { from.1 } else { 0 };
            let end = if i == to.0 { (to.1 + 1).min(cols) } else { cols };
            let mut piece = String::new();
            for c in start..end {
                push_cell(&mut piece, &cells[Column(c)]);
            }
            if i < to.0 && cells[Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
                out.push_str(&piece);
            } else {
                out.push_str(piece.trim_end());
                if i < to.0 {
                    out.push('\n');
                }
            }
        }
        out
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

    /// The cells of row `i`, 0 being the oldest history line.
    fn line(&self, i: usize) -> Option<&alacritty_terminal::grid::Row<Cell>> {
        let line = Line(i as i32 - self.history() as i32);
        let grid = self.term.grid();
        (line >= grid.topmost_line() && line <= grid.bottommost_line()).then(|| &grid[line])
    }

    /// Up to the last cell holding something visible.
    fn end_of(&self, cells: &alacritty_terminal::grid::Row<Cell>) -> usize {
        (0..self.term.grid().columns()).rev().find(|c| !is_blank(&cells[Column(*c)])).map_or(0, |c| c + 1)
    }

    /// Row `i`, 0 being the oldest history line.
    pub fn row(&self, i: usize) -> Row {
        let Some(cells) = self.line(i) else { return Row::default() };
        let mut row = Row::default();
        for c in 0..self.end_of(cells) {
            let cell = &cells[Column(c)];
            if is_spacer(cell) {
                continue;
            }
            let style = style_of(cell);
            let start = row.text.len();
            push_cell(&mut row.text, cell);
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

    /// The links on rows `rows` (numbered as [`Terminal::row`]): addresses a program
    /// marked (OSC 8), and `http://`, `https://` and `file://` addresses in the text. A
    /// link can run on over rows that wrapped; one starting above `rows` counts.
    pub fn links(&self, rows: std::ops::Range<usize>) -> Vec<Link> {
        let cols = self.term.grid().columns();
        let wraps = |i: usize| self.line(i).is_some_and(|cells| cells[Column(cols - 1)].flags.contains(Flags::WRAPLINE));
        // From the start of the line the first row belongs to.
        let mut i = rows.start;
        while i > 0 && wraps(i - 1) {
            i -= 1;
        }
        let mut out = Vec::new();
        while i < rows.end {
            // One line as the text reads, and the cell each character came from.
            let (mut text, mut cells, mut marked) = (String::new(), Vec::new(), Vec::new());
            while let Some(row) = self.line(i) {
                let end = if wraps(i) { cols } else { self.end_of(row) };
                for c in 0..end {
                    let cell = &row[Column(c)];
                    if is_spacer(cell) {
                        continue;
                    }
                    let before = text.len();
                    push_cell(&mut text, cell);
                    for _ in before..text.len() {
                        cells.push((i, c));
                    }
                    marked.push(((i, c), cell.hyperlink().map(|h| h.uri().to_string())));
                }
                i += 1;
                if !wraps(i - 1) {
                    break;
                }
            }
            if cells.is_empty() && self.line(i).is_none() {
                break;
            }
            let from_marks = marked_links(&marked);
            for link in from_marks.iter().chain(&text_links(&text, &cells)) {
                if !out.iter().any(|o: &Link| o.from == link.from) && link.from.0 < rows.end && link.to.0 >= rows.start {
                    out.push(link.clone());
                }
            }
        }
        out
    }
}

/// A link in a terminal: from cell `from` to cell `to` (row, column), both included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub from: (usize, usize),
    pub to: (usize, usize),
    pub url: String,
}

/// Addresses that open: on the web, or a file.
const SCHEMES: [&str; 3] = ["https://", "http://", "file://"];

/// Runs of cells a program marked with the same address that opens. Output decides
/// what is marked: an address that doesn't open is skipped over all its cells.
fn marked_links(marked: &[((usize, usize), Option<String>)]) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    // The address of the run the previous cell is in, and whether that run was kept.
    let mut run: Option<(&str, bool)> = None;
    for (cell, url) in marked {
        run = match (url.as_deref(), run) {
            (Some(u), Some((r, kept))) if r == u => {
                if kept && let Some(last) = out.last_mut() {
                    last.to = *cell;
                }
                Some((r, kept))
            }
            (Some(u), _) => {
                let kept = SCHEMES.iter().any(|s| u.starts_with(s));
                if kept {
                    out.push(Link { from: *cell, to: *cell, url: u.to_string() });
                }
                Some((u, kept))
            }
            (None, _) => None,
        };
    }
    out
}

/// Addresses written in `text` (one line), `cells` giving each byte's cell. An address
/// ends at a space, a quote or `<>`; trailing punctuation isn't part of it, nor a
/// closing bracket it didn't open.
fn text_links(text: &str, cells: &[(usize, usize)]) -> Vec<Link> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some((start, scheme)) = SCHEMES.iter().filter_map(|s| text[at..].find(s).map(|i| (at + i, *s))).min_by_key(|(i, _)| *i) {
        let preceded = text[..start].chars().next_back().is_some_and(|c| c.is_alphanumeric());
        let rest = &text[start..];
        let mut end = rest.find(|c: char| c.is_whitespace() || "\"'<>`".contains(c)).unwrap_or(rest.len());
        loop {
            let url = &rest[..end];
            let Some(last) = url.chars().next_back() else { break };
            let unopened = |open: char, close: char| last == close && url.matches(open).count() < url.matches(close).count();
            if ".,;:!?".contains(last) || unopened('(', ')') || unopened('[', ']') || unopened('{', '}') {
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        if !preceded && end > scheme.len() {
            out.push(Link { from: cells[start], to: cells[start + end - 1], url: rest[..end].to_string() });
        }
        at = start + end.max(scheme.len());
    }
    out
}

/// The second half of a wide character, or the gap a wide one left at a row's end.
fn is_spacer(cell: &Cell) -> bool {
    cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
}

fn push_cell(text: &mut String, cell: &Cell) {
    if is_spacer(cell) {
        return;
    }
    text.push(if cell.flags.contains(Flags::HIDDEN) || cell.c == '\0' { ' ' } else { cell.c });
    if let Some(extra) = cell.zerowidth() {
        text.extend(extra);
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

/// Modifier keys held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Super, the platform key.
    pub meta: bool,
}

/// The bytes a key sends to the process (xterm's encoding), or `None` when it sends none
/// and stays the app's: `alt+…` and `ctrl+shift+…` are always the app's (M2 decision), and
/// so are keys a terminal has no bytes for, like ctrl+tab or ctrl+digit. `key` is the
/// key's name (`a`, `enter`, `f5`), `text` what it types (`A` with shift); cursor keys
/// follow application-cursor mode (DECCKM).
pub fn key_bytes(key: &str, text: Option<&str>, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    if mods.alt || mods.meta || (mods.ctrl && mods.shift) {
        return None;
    }
    // xterm's modifier parameter: 1 + shift + 4·ctrl (alt never gets here).
    let m = 1 + mods.shift as u8 + 4 * mods.ctrl as u8;
    let cursor = |end: char| match (m, app_cursor) {
        (1, true) => format!("\x1bO{end}"),
        (1, false) => format!("\x1b[{end}"),
        _ => format!("\x1b[1;{m}{end}"),
    };
    let tilde = |n: u8| if m == 1 { format!("\x1b[{n}~") } else { format!("\x1b[{n};{m}~") };
    let ss3 = |end: char| if m == 1 { format!("\x1bO{end}") } else { format!("\x1b[1;{m}{end}") };
    let text = match key {
        "enter" => "\r".into(),
        "backspace" if mods.ctrl => "\x08".into(),
        "backspace" => "\x7f".into(),
        "tab" if mods.ctrl => return None,
        "tab" if mods.shift => "\x1b[Z".into(),
        "tab" => "\t".into(),
        "escape" => "\x1b".into(),
        "space" if mods.ctrl => "\0".into(),
        "space" => " ".into(),
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => ss3('P'),
        "f2" => ss3('Q'),
        "f3" => ss3('R'),
        "f4" => ss3('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        _ if mods.ctrl => return ctrl_code(key).map(|b| vec![b]),
        _ => {
            let mut chars = key.chars();
            let typed = match (text, chars.next(), chars.next()) {
                (Some(t), _, _) => t.to_string(),
                (None, Some(c), None) if mods.shift => c.to_uppercase().collect(),
                (None, Some(c), None) => c.to_string(),
                _ => return None,
            };
            if typed.is_empty() || typed.chars().any(char::is_control) {
                return None;
            }
            typed
        }
    };
    Some(text.into_bytes())
}

/// The control code of ctrl+`key`: letters, and the symbols ASCII pairs with codes.
fn ctrl_code(key: &str) -> Option<u8> {
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return None };
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' => Some(0),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' | '/' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

/// What pasting `text` sends: line breaks become ⏎ (`\r`), and in bracketed-paste mode
/// the text goes between the markers, without escape characters that could end it early.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes()
    } else {
        text.into_bytes()
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
    fn wide_characters_take_two_columns_and_wrap_whole() {
        let mut t = term(5, 5);
        t.feed("a家b".as_bytes());
        assert_eq!(t.lines(), ["a家b"]);
        // One column left: the next wide one goes to the next row whole.
        t.feed("家".as_bytes());
        assert_eq!(t.lines(), ["a家b", "家"]);
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

    #[test]
    fn colour_queries_get_the_look_and_colours_the_program_set() {
        let mut t = term(20, 5);
        set_look(Look { fg: (0x10, 0x20, 0x30), bg: (1, 2, 3), ..Look::DEFAULT });
        t.feed(b"\x1b]11;?\x07\x1b]10;?\x1b\\");
        assert_eq!(t.take_replies(), b"\x1b]11;rgb:0101/0202/0303\x07\x1b]10;rgb:1010/2020/3030\x1b\\");
        t.feed(b"\x1b]4;1;rgb:ff/00/00\x07\x1b]4;1;?\x07\x1b]4;2;?\x07");
        let replies = String::from_utf8(t.take_replies()).unwrap();
        assert_eq!(replies, "\x1b]4;1;rgb:ffff/0000/0000\x07\x1b]4;2;rgb:0000/cdcd/0000\x07", "set by the program, then the look's");
        set_look(Look::DEFAULT);
    }

    #[test]
    fn size_queries_answer_in_cells_and_pixels_in_order() {
        let mut t = term(20, 5);
        set_look(Look { cell: (9, 18), ..Look::DEFAULT });
        t.feed(b"\x1b[14t\x1b[18t\x1b[6n");
        assert_eq!(t.take_replies(), b"\x1b[4;90;180t\x1b[8;5;20t\x1b[1;1R");
        set_look(Look::DEFAULT);
    }

    #[test]
    fn modes_and_cursor_follow_the_program() {
        let mut t = term(20, 5);
        assert_eq!(t.modes(), Modes { show_cursor: true, alternate_scroll: true, ..Modes::default() });
        t.feed(b"\x1b[?1h\x1b[?2004h\x1b[?25l");
        let m = t.modes();
        assert!(m.app_cursor && m.bracketed_paste && !m.show_cursor && !m.alt_screen);
        t.feed(b"\x1b[?1049h");
        assert!(t.modes().alt_screen);
        t.feed(b"\x1b[?1049l\x1b[?1l\x1b[?25h");
        assert!(!t.modes().alt_screen && !t.modes().app_cursor && t.modes().show_cursor);
    }

    #[test]
    fn the_cursor_knows_its_row_column_and_byte() {
        let mut t = term(10, 3);
        t.feed(b"a\r\nb\r\nc\r\nd\xc3\xa9f\x1b[2D");
        let c = t.cursor();
        assert_eq!((c.row, c.col), (3, 1), "one row went to the history");
        assert_eq!(c.byte, Some(1));
        assert_eq!(&t.row(c.row).text[1..3], "é");
        t.feed(b"\x1b[5G");
        assert_eq!(t.cursor().byte, None, "past the text");
    }

    #[test]
    fn selected_text_joins_wrapped_rows_and_trims_line_ends() {
        let mut t = term(5, 5);
        t.feed(b"abcdefg\r\nhi   \r\n\r\nxyz");
        assert_eq!(t.lines(), ["abcde", "fg", "hi", "", "xyz"]);
        assert_eq!(t.text((0, 2), (1, 4)), "cdefg", "a wrapped row runs on");
        assert_eq!(t.text((1, 0), (4, 1)), "fg\nhi\n\nxy");
        assert_eq!(t.text((4, 1), (2, 1)), "i\n\nxy", "either direction");
    }

    #[test]
    fn dropped_rows_are_counted_so_rows_keep_their_number() {
        let mut t = Terminal::new(TermSize { cols: 20, rows: 3 }, 5);
        for i in 0..20 {
            t.feed(format!("line {i}\r\n").as_bytes());
        }
        assert_eq!(t.dropped(), 13, "21 rows written, 3 on screen, 5 kept");
        assert_eq!(t.row(0).text, format!("line {}", t.dropped()));
        t.feed(b"a\r\nb\r\nc\r\n");
        assert_eq!(t.dropped(), 16);
        assert_eq!(t.row(0).text, "line 16");
        t.feed(b"\x1b[?1049hfull screen\r\n\r\n\r\n\x1b[?1049l");
        assert_eq!(t.dropped(), 16, "the alternate screen has no history");
    }

    fn keys(key: &str, text: Option<&str>, mods: Mods) -> Option<String> {
        key_bytes(key, text, mods, false).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn keys_become_xterm_bytes() {
        let none = Mods::default();
        let ctrl = Mods { ctrl: true, ..none };
        let shift = Mods { shift: true, ..none };
        for (key, text, mods, out) in [
            ("a", Some("a"), none, "a"),
            ("a", Some("A"), shift, "A"),
            ("é", Some("é"), none, "é"),
            ("enter", None, none, "\r"),
            ("backspace", None, none, "\x7f"),
            ("backspace", None, ctrl, "\x08"),
            ("tab", None, none, "\t"),
            ("tab", None, shift, "\x1b[Z"),
            ("escape", None, none, "\x1b"),
            ("space", Some(" "), none, " "),
            ("space", None, ctrl, "\0"),
            ("up", None, none, "\x1b[A"),
            ("left", None, ctrl, "\x1b[1;5D"),
            ("end", None, shift, "\x1b[1;2F"),
            ("pageup", None, none, "\x1b[5~"),
            ("delete", None, ctrl, "\x1b[3;5~"),
            ("insert", None, none, "\x1b[2~"),
            ("f1", None, none, "\x1bOP"),
            ("f4", None, shift, "\x1b[1;2S"),
            ("f5", None, none, "\x1b[15~"),
            ("f12", None, ctrl, "\x1b[24;5~"),
            ("c", None, ctrl, "\x03"),
            ("z", Some("z"), ctrl, "\x1a"),
            ("[", None, ctrl, "\x1b"),
            ("/", None, ctrl, "\x1f"),
            ("?", None, none, "?"),
            ("n", None, shift, "N"),
        ] {
            assert_eq!(keys(key, text, mods).as_deref(), Some(out), "{key} {mods:?}");
        }
    }

    #[test]
    fn alt_ctrl_shift_and_keys_without_bytes_stay_the_apps() {
        let ctrl = Mods { ctrl: true, ..Mods::default() };
        for (key, mods) in [
            ("f", Mods { alt: true, ..Mods::default() }),
            ("left", Mods { alt: true, ..Mods::default() }),
            ("c", Mods { ctrl: true, shift: true, ..Mods::default() }),
            ("enter", Mods { ctrl: true, shift: true, ..Mods::default() }),
            ("a", Mods { meta: true, ..Mods::default() }),
            ("tab", ctrl),
            ("1", ctrl),
            ("=", ctrl),
            ("-", ctrl),
            (",", ctrl),
            ("menu", Mods::default()),
        ] {
            assert_eq!(keys(key, None, mods), None, "{key} {mods:?}");
        }
    }

    #[test]
    fn cursor_keys_follow_application_cursor_mode() {
        let b = |key, mods| key_bytes(key, None, mods, true).map(|b| String::from_utf8(b).unwrap());
        assert_eq!(b("up", Mods::default()).as_deref(), Some("\x1bOA"));
        assert_eq!(b("home", Mods::default()).as_deref(), Some("\x1bOH"));
        assert_eq!(b("up", Mods { shift: true, ..Mods::default() }).as_deref(), Some("\x1b[1;2A"), "modified keys keep CSI");
    }

    #[test]
    fn pastes_send_returns_and_honour_bracketed_paste() {
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(paste_bytes("x\x1b[201~y\n", true), b"\x1b[200~x[201~y\r\x1b[201~", "no way out of the brackets");
    }

    #[test]
    fn links_are_found_in_the_text_and_where_programs_marked_them() {
        let mut t = term(30, 6);
        t.feed(b"  Local:   http://localhost:5173/\r\n");
        t.feed(b"see (https://x.dev/a_(b)) and https://y.dev/p?q=1, done.\r\n");
        t.feed(b"\x1b]8;;https://docs.rs/gpui\x1b\\docs\x1b]8;;\x1b\\ and mailto:a@b.c ftp://x\r\n");
        let links = t.links(0..t.rows());
        let urls: Vec<&str> = links.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(urls, ["http://localhost:5173/", "https://x.dev/a_(b)", "https://y.dev/p?q=1", "https://docs.rs/gpui"]);
        assert_eq!((links[0].from, links[0].to), ((0, 11), (1, 2)), "33 characters wrap at 30");
        let docs = links.iter().find(|l| l.url == "https://docs.rs/gpui").unwrap();
        assert_eq!((docs.from, docs.to), ((4, 0), (4, 3)), "the marked text, not the address");
    }

    #[test]
    fn marked_addresses_that_dont_open_are_skipped_whole() {
        let mut t = term(40, 4);
        t.feed(b"\x1b]8;;javascript:alert(1)\x1b\\click me\x1b]8;;\x1b\\ \x1b]8;;https://a.dev\x1b\\a\x1b]8;;\x1b\\\r\n");
        let links = t.links(0..t.rows());
        assert_eq!(links.iter().map(|l| l.url.as_str()).collect::<Vec<_>>(), ["https://a.dev"], "no panic, nothing else kept");
        assert_eq!((links[0].from, links[0].to), ((0, 9), (0, 9)));
    }

    #[test]
    fn a_link_that_wraps_is_one_link() {
        let mut t = term(16, 5);
        t.feed(b"go to https://example.com/a/long/path now\r\n");
        let links = t.links(1..2);
        assert_eq!(links.len(), 1, "found from its second row: {links:?}");
        assert_eq!(links[0].url, "https://example.com/a/long/path");
        assert_eq!((links[0].from, links[0].to), ((0, 6), (2, 4)));
        assert_eq!(t.links(4..5), [], "rows without links");
    }
}
