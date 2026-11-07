//! Theme tokens (R9.2, R9.3). Colors come from the theme; every size is a base value
//! multiplied by the UI scale (R9.4) and rounded once here, to whole logical pixels,
//! matching how GPUI rounds line heights. Only the hairline stays 1 px at any scale.

use gpui::{App, Global, Hsla, Pixels, SharedString, Window, px, rgb};

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: SharedString,
    pub scale: f32,
    // colors
    pub bg: Hsla,
    pub bar: Hsla,
    pub raised: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
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
    pub env: Hsla,
    pub env_bg: Hsla,
    // type
    pub font: SharedString,
    pub text: Pixels,
    pub text_small: Pixels,
    pub line_height: Pixels,
    // sizes
    pub row_h: Pixels,
    pub tab_h: Pixels,
    pub rail_w: Pixels,
    pub title_h: Pixels,
    pub status_h: Pixels,
    pub toolbar_h: Pixels,
    pub button_h: Pixels,
    pub icon_button: Pixels,
    pub grip_w: Pixels,
    pub tab_min_w: Pixels,
    pub tab_max_w: Pixels,
    pub tabs_min_w: Pixels,
    pub divider_grab: Pixels,
    pub window_edge: Pixels,
    pub radius_s: Pixels,
    pub radius_m: Pixels,
    pub hairline: Pixels,
}

impl Global for Theme {}

impl Theme {
    /// The approved prototype palette with slightly more contrast (docs/DESIGN.md).
    pub fn devdeck_dark(scale: f32) -> Self {
        let c = |v: u32| -> Hsla { rgb(v).into() };
        let mut drop_bg: Hsla = rgb(0x64a6e3).into();
        drop_bg.a = 0.18;
        let s = |base: f32| scaled(base, scale);
        Self {
            name: "devdeck-dark".into(),
            scale,
            bg: c(0x0f1012),
            bar: c(0x15161a),
            raised: c(0x1c1d22),
            hover: c(0x1b1e24),
            selected: c(0x1f2c3d),
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
            env: c(0xcbbdf7),
            env_bg: c(0x2d2645),
            font: "JetBrains Mono NL".into(),
            text: s(13.),
            text_small: s(11.),
            line_height: s(20.),
            row_h: s(24.),
            tab_h: s(32.),
            rail_w: s(28.),
            title_h: s(34.),
            status_h: s(24.),
            toolbar_h: s(28.),
            button_h: s(24.),
            icon_button: s(22.),
            grip_w: s(18.),
            tab_min_w: s(60.),
            tab_max_w: s(280.),
            tabs_min_w: s(72.),
            divider_grab: s(5.).max(px(5.)),
            window_edge: px(6.),
            radius_s: s(3.),
            radius_m: s(4.),
            hairline: px(1.),
        }
    }

    /// Any other size, scaled and rounded like the tokens.
    pub fn size(&self, base: f32) -> Pixels {
        scaled(base, self.scale)
    }

    /// The rem size for this scale, so GPUI's rem-based helpers (`px_3`, `gap_2`…)
    /// follow the UI scale too.
    pub fn rem(&self) -> Pixels {
        px(16. * self.scale)
    }
}

pub fn scaled(base: f32, scale: f32) -> Pixels {
    px((base * scale).round())
}

pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}

/// Applies the theme's scale to a window. Every root view calls this in `render`.
pub fn apply_window_scale(window: &mut Window, theme: &Theme) {
    window.set_rem_size(theme.rem());
}

/// Unscaled logical pixels, for OS window geometry (not UI content).
pub fn logical(v: f32) -> Pixels {
    px(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_scale_and_round() {
        let a = Theme::devdeck_dark(1.0);
        let b = Theme::devdeck_dark(1.5);
        let c = Theme::devdeck_dark(1.1);
        assert_eq!(a.row_h, px(24.));
        assert_eq!(b.row_h, px(36.));
        assert_eq!(c.row_h, px(26.), "24 × 1.1 = 26.4 rounds to 26");
        assert_eq!(c.text, px(14.), "13 × 1.1 = 14.3 rounds to 14");
        assert_eq!(b.hairline, px(1.), "hairlines never scale");
        assert_eq!(Theme::devdeck_dark(0.8).divider_grab, px(5.), "grab area has a floor");
        assert_eq!(b.rem(), px(24.));
        assert_eq!(b.size(10.), px(15.));
    }
}
