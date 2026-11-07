//! Icons: Lucide (lucide-static 1.61.0, ISC, `assets/icons/LICENSE`), only the ones
//! used, built into the binary. GPUI draws an SVG in one color, the one given to it, so
//! icons follow the theme like text. Adding one: copy its SVG from the same release into
//! `assets/icons/` and add a variant below (a missing file fails the build).
//!
//! The app serves them with [`Assets`]; tests without it draw nothing in their place.

use crate::theme::Theme;
use gpui::{Animation, AnimationElement, AnimationExt, AssetSource, ElementId, Hsla, Pixels, SharedString, Styled, Svg, Transformation, percentage, svg};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::time::Duration;

macro_rules! icons {
    ($($(#[$doc:meta])* $name:ident => $file:literal,)*) => {
        /// An icon of the set, by what it means here.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($(#[$doc])* $name,)*
        }

        impl Icon {
            pub const ALL: &[Icon] = &[$(Icon::$name,)*];

            /// Its path in [`Assets`].
            pub fn path(self) -> &'static str {
                match self {
                    $(Icon::$name => concat!("icons/", $file, ".svg"),)*
                }
            }

            fn svg(self) -> &'static [u8] {
                match self {
                    $(Icon::$name => include_bytes!(concat!("../assets/icons/", $file, ".svg")),)*
                }
            }
        }
    };
}

icons! {
    // Sidebar activities.
    Apps => "boxes",
    Cluster => "cloud",
    Ports => "ethernet-port",
    Layouts => "panels-top-left",
    Terminals => "square-terminal",
    // Runs.
    Run => "play",
    Stop => "square",
    Restart => "rotate-cw",
    /// A command's output: its terminal.
    Output => "square-terminal",
    /// Something in progress (stopping, rolling out, loading); see [`spinner`].
    Busy => "loader-circle",
    // States and messages.
    Check => "check",
    Ok => "circle-check",
    Warning => "triangle-alert",
    Error => "circle-x",
    Info => "info",
    /// A protected env: changes need a confirmation.
    Protected => "lock",
    // Controls.
    Close => "x",
    Open => "chevron-down",
    Expand => "chevron-right",
    Collapse => "chevron-up",
    Submenu => "chevron-right",
    Maximize => "maximize-2",
    Restore => "minimize-2",
    Send => "forward",
    Detach => "square-arrow-out-up-right",
    Reattach => "square-arrow-down-left",
    Minus => "minus",
    Plus => "plus",
    Grip => "grip-vertical",
    Settings => "settings",
    To => "arrow-right",
    // Window controls.
    WindowMinimize => "minus",
    WindowMaximize => "square",
    WindowRestore => "copy",
}

/// Serves the icons to GPUI (`Application::with_assets`).
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::ALL.iter().find(|i| i.path() == path).map(|i| Cow::Borrowed(i.svg())))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let paths: BTreeSet<&str> = Icon::ALL.iter().map(|i| i.path()).filter(|p| p.starts_with(path)).collect();
        Ok(paths.into_iter().map(SharedString::from).collect())
    }
}

/// `icon` at the theme's icon size, in `color`.
pub fn icon(icon: Icon, color: Hsla, t: &Theme) -> Svg {
    svg().path(icon.path()).size(t.icon).flex_none().text_color(color)
}

/// [`Icon::Busy`], `size` wide, turning once a second. It redraws while shown: only for
/// what lasts seconds (a run stopping), never for a state that can stay.
pub fn spinner(id: impl Into<ElementId>, color: Hsla, size: Pixels, t: &Theme) -> AnimationElement<Svg> {
    icon(Icon::Busy, color, t).size(size).with_animation(id, Animation::new(Duration::from_secs(1)).repeat(), |s, delta| {
        s.with_transformation(Transformation::rotate(percentage(delta)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_served_and_is_a_plain_outline() {
        for i in Icon::ALL {
            let svg = Assets.load(i.path()).unwrap().unwrap_or_else(|| panic!("{i:?} not served"));
            let text = std::str::from_utf8(&svg).unwrap();
            assert!(text.contains(r#"viewBox="0 0 24 24""#), "{i:?}: Lucide's 24 grid");
            for tag in ["<image", "<text", "<use", "<style", "<script"] {
                assert!(!text.contains(tag), "{i:?}: only shapes ({tag})");
            }
        }
        assert_eq!(Assets.load("icons/nope.svg").unwrap(), None);
        assert_eq!(Assets.list("icons/").unwrap().len(), Icon::ALL.iter().map(|i| i.path()).collect::<BTreeSet<_>>().len(), "each file once");
    }
}
