//! Theme tokens. Every color and size the UI uses comes from here (R9.2, R9.3).
//! M1 moves the values into theme files; the token names stay.

use gpui::{App, Global, Hsla, Pixels, SharedString, px, rgb};

#[derive(Clone)]
pub struct Theme {
    pub bg: Hsla,
    pub bar: Hsla,
    pub raised: Hsla,
    pub hover: Hsla,
    pub line: Hsla,
    pub line_soft: Hsla,
    pub fg: Hsla,
    pub hi: Hsla,
    pub mute: Hsla,
    pub dim: Hsla,
    pub accent: Hsla,
    pub accent_bg: Hsla,
    pub drop_bg: Hsla,
    pub ok: Hsla,
    pub warn: Hsla,
    pub err: Hsla,
    pub font: SharedString,
    pub text_size: Pixels,
    pub line_height: Pixels,
    pub tab_h: Pixels,
    pub rail_w: Pixels,
    pub title_h: Pixels,
    pub status_h: Pixels,
}

impl Global for Theme {}

impl Theme {
    /// The approved prototype palette with slightly more contrast (docs/DESIGN.md).
    pub fn devdeck_dark() -> Self {
        let c = |v: u32| -> Hsla { rgb(v).into() };
        let mut drop_bg: Hsla = rgb(0x64a6e3).into();
        drop_bg.a = 0.18;
        Self {
            bg: c(0x0f1012),
            bar: c(0x15161a),
            raised: c(0x1c1d22),
            hover: c(0x1b1e24),
            line: c(0x30333a),
            line_soft: c(0x23252b),
            fg: c(0xe1e2e5),
            hi: c(0xf5f6f8),
            mute: c(0x8d9099),
            dim: c(0x666a73),
            accent: c(0x64a6e3),
            accent_bg: c(0x16222f),
            drop_bg,
            ok: c(0x67c174),
            warn: c(0xe3ad48),
            err: c(0xeb7474),
            font: "JetBrains Mono NL".into(),
            text_size: px(13.),
            line_height: px(20.),
            tab_h: px(32.),
            rail_w: px(28.),
            title_h: px(34.),
            status_h: px(24.),
        }
    }
}

pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}
