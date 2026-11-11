//! Scroll containers (R9.6): a gesture moves one axis, the one it started on, so a pane
//! that scrolls both ways never drifts diagonally. A sideways-only area scrolls with a
//! sideways gesture or shift+wheel, not the plain wheel. Every scroll container of the
//! app goes through these (`clippy.toml` disallows GPUI's own outside this crate).

use gpui::{App, ElementId, InteractiveElement, IntoElement, Overflow, Stateful, UniformList, Window};
use std::ops::Range;

pub trait Scrolls: InteractiveElement + Sized {
    /// Scrolls up and down.
    fn scroll_y(self) -> Self {
        self.scrolls(false, true)
    }

    /// Scrolls sideways.
    fn scroll_x(self) -> Self {
        self.scrolls(true, false)
    }

    /// A strip (tabs): sideways only, and the plain wheel scrolls it sideways too.
    fn scroll_strip(mut self) -> Self {
        let style = &mut self.interactivity().base_style;
        style.overflow.x = Some(Overflow::Scroll);
        style.restrict_scroll_to_axis = Some(false);
        self
    }

    /// Scrolls both ways, one axis per gesture.
    fn scroll_xy(self) -> Self {
        self.scrolls(true, true)
    }

    fn scrolls(mut self, x: bool, y: bool) -> Self {
        let style = &mut self.interactivity().base_style;
        if x {
            style.overflow.x = Some(Overflow::Scroll);
        }
        if y {
            style.overflow.y = Some(Overflow::Scroll);
        }
        style.restrict_scroll_to_axis = Some(true);
        self
    }
}

/// Only elements with an id keep a scroll offset, so a plain `div` must be given one first.
impl<E: InteractiveElement> Scrolls for Stateful<E> {}
impl Scrolls for UniformList {}

/// GPUI's `uniform_list` with the axis lock: it scrolls up and down (a wide one adds
/// `with_horizontal_sizing_behavior(Unconstrained)`, and scrolls sideways too).
pub fn scroll_list<R: IntoElement>(
    id: impl Into<ElementId>,
    count: usize,
    rows: impl 'static + Fn(Range<usize>, &mut Window, &mut App) -> Vec<R>,
) -> UniformList {
    gpui::uniform_list(id, count, rows).scroll_y()
}
