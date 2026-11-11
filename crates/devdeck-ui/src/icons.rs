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
    /// Pulses: a heartbeat. From gpui-kit-assets' Lucide set.
    Layouts => "activity",
    Terminals => "square-terminal",
    /// A group the user made in a list.
    Group => "folder",
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
    /// Back to what was shown before (a full view's details). From gpui-kit-assets' Lucide set.
    Back => "arrow-left",
    /// The corner of a "go there" icon ([`crate::go_action`]). From gpui-kit-assets' Lucide set.
    GoCorner => "arrow-up-right",
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
    /// A Markdown file or a note shown rendered (R12.4). From gpui-kit-assets' Lucide set.
    Rendered => "eye",
    File => "file-text",
    Note => "sticky-note",
    To => "arrow-right",
    // What a tab shows (with File, Note, Apps, Cluster, Ports, Warning, Output, Run).
    /// A git panel.
    Git => "git-branch",
    /// A service's logs.
    Logs => "scroll-text",
    /// A pod.
    Pod => "box",
    /// The branch matrix.
    Matrix => "table-2",
    /// A code file.
    FileCode => "file-code",
    // The git panel's buttons.
    Fetch => "download",
    Pull => "arrow-down-to-line",
    Push => "arrow-up-from-line",
    NewBranch => "git-branch-plus",
    Refresh => "refresh-cw",
    // Logs.
    /// What level of lines shows.
    Filter => "funnel",
    /// Long lines wrap at the pane's width.
    Wrap => "text-wrap",
    Pause => "pause",
    /// Keep the newest line in view. From gpui-kit-assets' Lucide set.
    Follow => "chevrons-down",
    // Toolbars.
    /// A toolbar's "more" menu (never the settings cog). From gpui-kit-assets' Lucide set.
    More => "ellipsis",
    /// A toolbar's field shown as a button at its narrowest. From gpui-kit-assets' Lucide set.
    Search => "search",
    // Notifications. From gpui-kit-assets' Lucide set.
    /// The notification center (the title bar's bell).
    Bell => "bell",
    /// Mark notifications read.
    MarkRead => "check-check",
    /// Remove notifications.
    Clear => "trash",
    /// Only the unread notifications.
    Unread => "mail",
    /// A notify dropdown with nothing ticked.
    BellOff => "bell-off",
    // Deploys.
    /// Deploy the tag typed (the service view's tag row).
    Deploy => "rocket",
    // Window controls.
    WindowMinimize => "minus",
    WindowMaximize => "square",
    WindowRestore => "copy",
}

/// What the app is called wherever it names itself (the installed launcher's name;
/// the binary and the files stay `devdeck`).
pub const APP_NAME: &str = "Kugelblitz";

/// The app's icon, in colour (drawn with `img`, not tinted like the icons above): the
/// installed launcher's (`assets/app/devdeck.svg`).
pub const APP_ICON: &str = "app/kugelblitz.svg";
const APP_ICON_SVG: &[u8] = include_bytes!("../../../assets/app/devdeck.svg");

/// Serves the icons to GPUI (`Application::with_assets`).
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON {
            return Ok(Some(Cow::Borrowed(APP_ICON_SVG)));
        }
        match Icon::ALL.iter().find(|i| i.path() == path) {
            Some(i) => Ok(Some(Cow::Borrowed(i.svg()))),
            // The editor's find bar draws gpui-component's own icons: Lucide too (R9.5).
            None => Ok(gpui_kit_assets::Assets.load(path).ok().flatten()),
        }
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let paths: BTreeSet<&str> = Icon::ALL.iter().map(|i| i.path()).chain([APP_ICON]).filter(|p| p.starts_with(path)).collect();
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
