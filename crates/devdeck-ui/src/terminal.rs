//! Drawing a terminal's rows (R9.3): colors from the theme's `[ansi]` table, every row at
//! the monospace cell grid, so the process's columns line up.

use crate::Theme;
use devdeck_core::terminal::{Color, Row, TermSize};
use gpui::{
    FontStyle, FontWeight, HighlightStyle, Hsla, Pixels, Size, StrikethroughStyle, StyledText, UnderlineStyle, Window, font, rgb,
    size,
};

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

/// One row as styled text.
pub fn row_text(row: &Row, t: &Theme) -> StyledText {
    let highlights = row.spans.iter().filter(|s| s.style != Default::default()).map(|s| {
        let st = s.style;
        let mut fg = color(st.fg, t);
        if st.dim {
            fg = fg.opacity(0.6);
        }
        let style = HighlightStyle {
            color: (st.fg != Color::Fg || st.dim).then_some(fg),
            background_color: (st.bg != Color::Bg).then(|| color(st.bg, t)),
            font_weight: st.bold.then_some(FontWeight::BOLD),
            font_style: st.italic.then_some(FontStyle::Italic),
            underline: st.underline.then_some(UnderlineStyle { thickness: t.hairline, color: Some(fg), wavy: false }),
            strikethrough: st.strike.then_some(StrikethroughStyle { thickness: t.hairline, color: Some(fg) }),
            ..Default::default()
        };
        (s.start..s.start + s.len, style)
    });
    StyledText::new(row.text.clone()).with_highlights(highlights)
}
