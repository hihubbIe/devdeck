//! Drawing a terminal (R9.3): colors from the theme's `[ansi]` table, every row at the
//! monospace cell grid, so the process's columns line up. Rows are shaped and painted
//! directly, one line each, without an element per row: a streaming terminal redraws
//! every frame. Also what a view needs around it: keys as bytes for the process, the
//! scroll position and the selection ([`Scrollback`]), and the look the terminals answer
//! colour and size queries with.

use crate::Theme;
use devdeck_core::terminal::{self as core, Color, Cursor, Look, Mods, Row, Span, TermSize};
use gpui::{
    App, Bounds, ContentMask, Font, FontStyle, FontWeight, Hsla, Keystroke, Pixels, Point, Rgba, Size, StrikethroughStyle, TextAlign, TextRun,
    UnderlineStyle, Window, actions, fill, font, point, rgb, size,
};

actions!(terminal, [Copy, Paste]);

/// The size of one character cell at the theme's text size.
pub fn cell_size(window: &Window, t: &Theme) -> Size<Pixels> {
    let ts = window.text_system();
    let id = ts.resolve_font(&font(t.font.clone()));
    let w = ts.advance(id, t.text, 'm').map(|s| s.width).unwrap_or(t.text * 0.6);
    size(w, t.line_height)
}

/// How many cells fit in `area`.
pub fn fit(area: Size<Pixels>, cell: Size<Pixels>) -> TermSize {
    let n = |len: Pixels, unit: Pixels| (f32::from(len) / f32::from(unit).max(1.)).floor().clamp(2., u16::MAX as f32) as u16;
    TermSize { cols: n(area.width, cell.width), rows: n(area.height, cell.height) }
}

pub fn color(c: Color, t: &Theme) -> Hsla {
    match c {
        Color::Fg => t.fg,
        Color::Bg => t.bg,
        Color::Ansi(n) => t.ansi[n as usize % 16],
        Color::Rgb(r, g, b) => rgb((r as u32) << 16 | (g as u32) << 8 | b as u32).into(),
    }
}

/// The bytes a keystroke sends to the process, `None` when the key stays devdeck's
/// (see [`core::key_bytes`]).
pub fn keystroke_bytes(k: &Keystroke, app_cursor: bool) -> Option<Vec<u8>> {
    let m = &k.modifiers;
    let mods = Mods { ctrl: m.control, alt: m.alt, shift: m.shift, meta: m.platform || m.function };
    core::key_bytes(&k.key, k.key_char.as_deref(), mods, app_cursor)
}

/// True when a focused terminal takes `k` for its process.
pub fn takes(k: &Keystroke) -> bool {
    keystroke_bytes(k, false).is_some()
}

/// Makes every terminal answer colour and size queries with the theme and `cell`.
pub fn sync_look(t: &Theme, cell: Size<Pixels>) {
    let rgb8 = |c: Hsla| {
        let c = Rgba::from(c);
        let v = |x: f32| (x * 255.).round().clamp(0., 255.) as u8;
        (v(c.r), v(c.g), v(c.b))
    };
    let look = Look {
        ansi: t.ansi.map(rgb8),
        fg: rgb8(t.fg),
        bg: rgb8(t.bg),
        cursor: rgb8(t.fg),
        cell: (f32::from(cell.width).round() as u16, f32::from(cell.height).round() as u16),
    };
    if core::look() != look {
        core::set_look(look);
    }
}

/// A cell of a terminal's output: its row counted from the first row the run ever wrote
/// (row `i` plus [`Terminal::dropped`](core::Terminal::dropped)), so it stays on its text
/// while old rows fall off the history; and its column.
pub type At = (u64, usize);

/// What a view keeps about the terminal it shows besides whether it follows the output
/// (the view's own `follow`): where it is scrolled to, and the selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scrollback {
    /// The first row shown when not following.
    top: u64,
    anchor: Option<At>,
    head: At,
    dragging: bool,
    moved: bool,
}

impl Scrollback {
    /// The first row to show, of `rows`, when `shown` fit.
    pub fn first(&self, follow: bool, rows: usize, dropped: u64, shown: usize) -> usize {
        let last = rows.saturating_sub(shown);
        if follow { last } else { (self.top.saturating_sub(dropped) as usize).min(last) }
    }

    /// Scrolls `lines` rows back (negative: forward). True when that reached the end, so
    /// the view follows again.
    pub fn scroll(&mut self, follow: bool, lines: isize, rows: usize, dropped: u64, shown: usize) -> bool {
        let last = rows.saturating_sub(shown);
        let first = (self.first(follow, rows, dropped, shown) as isize - lines).clamp(0, last as isize) as usize;
        self.top = dropped + first as u64;
        first == last
    }

    /// Keeps the rows shown now in view (when following stops).
    pub fn hold(&mut self, follow: bool, rows: usize, dropped: u64, shown: usize) {
        self.top = dropped + self.first(follow, rows, dropped, shown) as u64;
    }

    /// The mouse went down on `at`: a new selection starts there.
    pub fn press(&mut self, at: At) {
        (self.anchor, self.head, self.dragging, self.moved) = (Some(at), at, true, false);
    }

    /// The mouse moved to `at` while down. True when the selection changed.
    pub fn drag(&mut self, at: At) -> bool {
        if !self.dragging || at == self.head {
            return false;
        }
        (self.head, self.moved) = (at, true);
        true
    }

    /// The mouse went up: a click without a drag selects nothing.
    pub fn release(&mut self) {
        self.dragging = false;
        if !self.moved {
            self.anchor = None;
        }
    }

    pub fn clear(&mut self) {
        (self.anchor, self.dragging) = (None, false);
    }

    /// The selected cells, first and last (both included, in reading order), as rows of
    /// the terminal now. Rows that fell off the history are left out.
    pub fn selection(&self, dropped: u64) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.anchor.filter(|_| self.moved)?;
        let (from, to) = if anchor <= self.head { (anchor, self.head) } else { (self.head, anchor) };
        if to.0 < dropped {
            return None;
        }
        let from = if from.0 < dropped { (0, 0) } else { ((from.0 - dropped) as usize, from.1) };
        Some((from, ((to.0 - dropped) as usize, to.1)))
    }
}

/// Where a terminal was drawn: its origin, the cell size, the first row shown and the
/// grid, to turn mouse positions into cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub origin: Point<Pixels>,
    pub cell: Size<Pixels>,
    /// The first row shown, counted like [`At`].
    pub first: u64,
    /// The rows of the terminal and how many of them fit.
    pub rows: usize,
    pub shown: usize,
    pub cols: usize,
    pub dropped: u64,
}

impl Geometry {
    /// The cell under `pos`, kept inside the output.
    pub fn at(&self, pos: Point<Pixels>) -> At {
        let rel = pos - self.origin;
        let row = (f32::from(rel.y) / f32::from(self.cell.height)).floor().max(0.) as u64;
        let col = (f32::from(rel.x) / f32::from(self.cell.width)).floor().max(0.) as usize;
        let last = self.dropped + self.rows.saturating_sub(1) as u64;
        ((self.first + row).min(last), col.min(self.cols.saturating_sub(1)))
    }
}

/// What a terminal's area shows: rows from row `first`, the selection and the cursor (in
/// the terminal's row numbering), at `cols` columns.
pub struct Frame {
    pub rows: Vec<Row>,
    pub first: usize,
    pub cols: usize,
    pub selection: Option<((usize, usize), (usize, usize))>,
    pub cursor: Option<Cursor>,
}

/// Paints `frame` at `bounds.origin`, clipped to `bounds`: backgrounds, the selection, the
/// cursor (a block, its character in the background color), then the text.
pub fn paint(frame: Frame, bounds: Bounds<Pixels>, cell: Size<Pixels>, t: &Theme, window: &mut Window, cx: &mut App) {
    let fonts = Fonts::of(t);
    let (origin, first, cols, selection, cursor) = (bounds.origin, frame.first, frame.cols, frame.selection, frame.cursor);
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for (i, row) in frame.rows.into_iter().enumerate() {
            let r = first + i;
            let at = point(origin.x, origin.y + cell.height * i as f32);
            let cell_at = |col: usize| point(at.x + cell.width * col as f32, at.y);
            let cursor = cursor.filter(|c| c.row == r);
            let under = cursor.and_then(|c| c.byte).map(|b| b..b + row.text[b..].chars().next().map_or(0, char::len_utf8));
            let line = (!row.text.is_empty()).then(|| {
                let runs = runs(&row.spans, under, t, &fonts);
                window.text_system().shape_line(row.text.into(), t.text, &runs, None)
            });
            if let Some(line) = &line {
                let _ = line.paint_background(at, cell.height, TextAlign::Left, None, window, cx);
            }
            if let Some(((r0, c0), (r1, c1))) = selection
                && (r0..=r1).contains(&r)
            {
                let (a, b) = (if r == r0 { c0 } else { 0 }, if r == r1 { c1 + 1 } else { cols });
                window.paint_quad(fill(Bounds::new(cell_at(a), size(cell.width * b.saturating_sub(a) as f32, cell.height)), t.selection));
            }
            if let Some(c) = cursor {
                window.paint_quad(fill(Bounds::new(cell_at(c.col), cell), t.fg));
            }
            if let Some(line) = &line {
                let _ = line.paint(at, cell.height, TextAlign::Left, None, window, cx);
            }
        }
    });
}

/// The theme's font in the four styles a terminal uses.
struct Fonts {
    regular: Font,
    bold: Font,
    italic: Font,
    bold_italic: Font,
}

impl Fonts {
    fn of(t: &Theme) -> Self {
        let regular = font(t.font.clone());
        let bold = Font { weight: FontWeight::BOLD, ..regular.clone() };
        let italic = Font { style: FontStyle::Italic, ..regular.clone() };
        let bold_italic = Font { style: FontStyle::Italic, ..bold.clone() };
        Self { regular, bold, italic, bold_italic }
    }
}

/// A row's spans as text runs; the bytes in `invert` (the character under the cursor) get
/// the background color.
fn runs(spans: &[Span], invert: Option<std::ops::Range<usize>>, t: &Theme, fonts: &Fonts) -> Vec<TextRun> {
    let mut out = Vec::with_capacity(spans.len() + 2);
    for s in spans {
        let st = s.style;
        let mut fg = color(st.fg, t);
        if st.dim {
            fg = fg.opacity(0.6);
        }
        let run = |len: usize, color: Hsla| TextRun {
            len,
            font: match (st.bold, st.italic) {
                (false, false) => fonts.regular.clone(),
                (true, false) => fonts.bold.clone(),
                (false, true) => fonts.italic.clone(),
                (true, true) => fonts.bold_italic.clone(),
            },
            color,
            background_color: (st.bg != Color::Bg).then(|| self::color(st.bg, t)),
            underline: st.underline.then_some(UnderlineStyle { thickness: t.hairline, color: Some(fg), wavy: false }),
            strikethrough: st.strike.then_some(StrikethroughStyle { thickness: t.hairline, color: Some(fg) }),
        };
        let span = s.start..s.start + s.len;
        match &invert {
            Some(inv) if inv.start >= span.start && inv.end <= span.end && !inv.is_empty() => {
                for (len, c) in [(inv.start - span.start, fg), (inv.len(), t.bg), (span.end - inv.end, fg)] {
                    if len > 0 {
                        out.push(run(len, c));
                    }
                }
            }
            _ => out.push(run(s.len, fg)),
        }
    }
    out
}

/// The rows a wheel movement of `delta` pixels scrolls, keeping the remainder in `carry`.
pub fn wheel_rows(delta: Pixels, row: Pixels, carry: &mut f32) -> isize {
    let rows = f32::from(delta) / f32::from(row).max(1.) + *carry;
    let whole = rows.trunc();
    *carry = rows - whole;
    whole as isize
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn following_shows_the_end_and_scrolling_back_holds_the_rows() {
        let mut s = Scrollback::default();
        assert_eq!(s.first(true, 100, 0, 30), 70);
        assert!(!s.scroll(true, 10, 100, 0, 30));
        assert_eq!(s.first(false, 100, 0, 30), 60);
        assert_eq!(s.first(false, 120, 0, 30), 60, "stays put while output arrives");
        assert_eq!(s.first(false, 120, 5, 30), 55, "and while old rows fall off");
        assert!(s.scroll(false, -1000, 120, 5, 30), "back at the end");
        assert!(!s.scroll(false, 1000, 120, 5, 30));
        assert_eq!(s.first(false, 120, 5, 30), 0);
        assert_eq!(s.first(false, 10, 0, 30), 0, "short output");
    }

    #[test]
    fn holding_keeps_the_rows_in_view() {
        let mut s = Scrollback::default();
        s.hold(true, 100, 0, 30);
        assert_eq!(s.first(false, 150, 10, 30), 60);
        assert_eq!(s.first(true, 150, 10, 30), 120);
    }

    #[test]
    fn a_drag_selects_and_a_click_clears() {
        let mut s = Scrollback::default();
        s.press((10, 4));
        assert_eq!(s.selection(0), None, "nothing before a move");
        assert!(s.drag((8, 2)));
        assert!(!s.drag((8, 2)));
        s.release();
        assert_eq!(s.selection(0), Some(((8, 2), (10, 4))), "in reading order");
        assert_eq!(s.selection(9), Some(((0, 0), (1, 4))), "rows that fell off are cut");
        assert_eq!(s.selection(11), None);
        assert!(!s.drag((1, 1)), "only while the button is down");
        s.press((3, 3));
        s.release();
        assert_eq!(s.selection(0), None);
    }

    #[test]
    fn mouse_positions_map_to_cells_inside_the_output() {
        let g = Geometry { origin: point(px(10.), px(100.)), cell: size(px(8.), px(20.)), first: 50, rows: 60, shown: 30, cols: 80, dropped: 5 };
        assert_eq!(g.at(point(px(10.), px(100.))), (50, 0));
        assert_eq!(g.at(point(px(27.), px(141.))), (52, 2));
        assert_eq!(g.at(point(px(0.), px(0.))), (50, 0));
        assert_eq!(g.at(point(px(5000.), px(5000.))), (64, 79), "last row and column");
    }

    #[test]
    fn wheel_movements_add_up_to_rows() {
        let mut carry = 0.;
        assert_eq!(wheel_rows(px(30.), px(20.), &mut carry), 1);
        assert_eq!(wheel_rows(px(10.), px(20.), &mut carry), 1);
        assert_eq!(wheel_rows(px(-45.), px(20.), &mut carry), -2);
    }

    #[test]
    fn only_bindings_without_terminal_bytes_pass_through() {
        let k = |s: &'static str| takes(&Keystroke::parse(s).unwrap());
        for key in ["enter", "s", "r", "?", "ctrl-k", "ctrl-w", "ctrl-q", "shift-f10", "down", "escape"] {
            assert!(k(key), "{key} goes to the process");
        }
        for key in ["alt-f", "alt-left", "ctrl-shift-c", "ctrl-shift-tab", "ctrl-alt-1", "ctrl-1", "ctrl-tab", "ctrl-=", "ctrl--", "ctrl-,", "menu"] {
            assert!(!k(key), "{key} stays devdeck's");
        }
    }

    #[test]
    fn the_cursor_character_is_drawn_in_the_background_color() {
        let t = Theme::devdeck_dark(1.);
        let fonts = Fonts::of(&t);
        let spans = [Span { start: 0, len: 5, style: Default::default() }];
        let r = runs(&spans, Some(2..3), &t, &fonts);
        assert_eq!(r.iter().map(|r| (r.len, r.color == t.bg)).collect::<Vec<_>>(), [(2, false), (1, true), (2, false)]);
        assert_eq!(runs(&spans, None, &t, &fonts).len(), 1);
    }
}
