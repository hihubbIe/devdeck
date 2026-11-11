//! A scrollbar over a scroll container's right edge (R9.6): a thumb whose height says how
//! much of the content shows and whose place says where; nothing when everything shows.
//! It reads the container's [`ScrollHandle`] as it paints (a wheel scroll redraws the
//! view), so it never lags. Dragging the thumb scrolls; a press on the track brings the
//! thumb there, then drags. It sits over the content (absolute, a fixed width): showing
//! or hiding it moves nothing (R9.10).
//!
//! Put it right after the container, in a `relative` parent: [`with_scrollbar`] does.
//! [`scrollbar_x`] is the same along a container's bottom edge (a tab strip): thinner, and
//! a press on it never reaches what's under it.

use crate::theme::theme;
use gpui::{
    App, Bounds, ContentMask, Corners, CursorStyle, Element, ElementId, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Position, ScrollHandle, Style, Styled,
    UniformListScrollHandle, Window, div, fill, point, relative, size,
};
use std::cell::Cell;
use std::rc::Rc;

/// The scrollbar of the container tracking `handle`.
pub fn scrollbar(id: impl Into<ElementId>, handle: &ScrollHandle) -> Scrollbar {
    Scrollbar { id: id.into(), handle: handle.clone(), across: false }
}

/// The scrollbar along the bottom edge of the container tracking `handle` (scrolling
/// sideways).
pub fn scrollbar_x(id: impl Into<ElementId>, handle: &ScrollHandle) -> Scrollbar {
    Scrollbar { id: id.into(), handle: handle.clone(), across: true }
}

/// A list's ([`crate::scroll_list`]) handle as [`scrollbar`] reads it.
pub fn list_handle(handle: &UniformListScrollHandle) -> ScrollHandle {
    handle.0.borrow().base_handle.clone()
}

/// `scroller` (tracking `handle`) with its scrollbar over its right edge; the pair takes
/// the room `scroller` would.
pub fn with_scrollbar(id: impl Into<ElementId>, scroller: impl IntoElement, handle: &ScrollHandle) -> gpui::Div {
    div().relative().flex().flex_col().min_h_0().min_w_0().child(scroller).child(scrollbar(id, handle))
}

pub struct Scrollbar {
    id: ElementId,
    handle: ScrollHandle,
    /// Along the bottom edge, for sideways scrolling.
    across: bool,
}

/// `b` with its axes swapped: the sideways bar works out its thumb as the upright one.
fn swap(b: Bounds<Pixels>) -> Bounds<Pixels> {
    Bounds { origin: point(b.origin.y, b.origin.x), size: size(b.size.height, b.size.width) }
}

/// A point's coordinate along the bar.
fn along(p: gpui::Point<Pixels>, across: bool) -> Pixels {
    if across { p.x } else { p.y }
}

/// Where a drag holds the thumb: the pointer's distance from its top.
#[derive(Clone, Default)]
struct Drag(Rc<Cell<Option<Pixels>>>);

/// The thumb in its track: its top and height, from what the handle says.
fn thumb(track: Bounds<Pixels>, viewport: Pixels, max: Pixels, offset: Pixels, min: Pixels) -> (Pixels, Pixels) {
    let content = viewport + max;
    let h = (track.size.height * (viewport / content)).max(min).min(track.size.height);
    let room = track.size.height - h;
    let at = if max > Pixels::ZERO { room * ((-offset) / max).clamp(0., 1.) } else { Pixels::ZERO };
    (track.top() + at, h)
}

/// The offset putting the thumb's top at `top`.
fn offset_for(track: Bounds<Pixels>, h: Pixels, max: Pixels, top: Pixels) -> Pixels {
    let room = track.size.height - h;
    if room <= Pixels::ZERO {
        return Pixels::ZERO;
    }
    -(max * ((top - track.top()) / room).clamp(0., 1.))
}

impl IntoElement for Scrollbar {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Scrollbar {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    /// The parent's right edge, top to bottom, a fixed width.
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let t = theme(cx);
        let mut style = Style { position: Position::Absolute, ..Style::default() };
        style.inset.bottom = Pixels::ZERO.into();
        style.inset.right = Pixels::ZERO.into();
        if self.across {
            style.inset.left = Pixels::ZERO.into();
            style.size.width = relative(1.).into();
            style.size.height = t.size(5.).into();
        } else {
            style.inset.top = Pixels::ZERO.into();
            style.size.width = t.size(10.).into();
            style.size.height = relative(1.).into();
        }
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, _: &mut App) -> Option<Hitbox> {
        if self.across {
            // A press on it never reaches the tab under it.
            return (self.handle.max_offset().x > Pixels::ZERO).then(|| window.insert_hitbox(bounds, HitboxBehavior::BlockMouse));
        }
        (self.handle.max_offset().y > Pixels::ZERO).then(|| window.insert_hitbox(bounds, HitboxBehavior::Normal))
    }

    fn paint(&mut self, id: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), hitbox: &mut Option<Hitbox>, window: &mut Window, cx: &mut App) {
        let Some(hitbox) = hitbox.clone() else { return };
        let t = theme(cx).clone();
        let across = self.across;
        let pick = move |p: gpui::Point<Pixels>| along(p, across);
        let (viewport, max, offset) = if across {
            (self.handle.bounds().size.width, self.handle.max_offset().x, self.handle.offset().x)
        } else {
            (self.handle.bounds().size.height, self.handle.max_offset().y, self.handle.offset().y)
        };
        let pad = t.size(2.);
        // Worked out upright; the sideways bar swaps its axes in and out.
        let upright = if across { swap(bounds) } else { bounds };
        let track = Bounds { origin: point(upright.left(), upright.top() + pad), size: size(upright.size.width, upright.size.height - pad * 2.) };
        let (top, h) = thumb(track, viewport, max, offset, t.size(24.));
        // One drag state across frames: the listeners of the next frame see a drag begun now.
        let drag = id
            .map(|id| {
                window.with_element_state(id, |state: Option<Drag>, _| {
                    let d = state.unwrap_or_default();
                    (d.clone(), d)
                })
            })
            .unwrap_or_default();
        let hovered = hitbox.is_hovered(window) || drag.0.get().is_some();
        let width = match (across, hovered) {
            (true, _) => t.size(3.),
            (false, true) => t.size(7.),
            (false, false) => t.size(5.),
        };
        let shape = Bounds { origin: point(upright.right() - width - if across { t.size(1.) } else { pad }, top), size: size(width, h) };
        let shape = if across { swap(shape) } else { shape };
        let color = if hovered { t.mute.opacity(0.6) } else { t.scrollbar };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(shape, color).corner_radii(Corners::all(width / 2.)));
        });
        window.set_cursor_style(CursorStyle::Arrow, &hitbox);

        let handle = self.handle.clone();
        let (down, hb) = (drag.clone(), hitbox.clone());
        let set = move |handle: &ScrollHandle, at: Pixels| {
            if across {
                handle.set_offset(point(offset_for(track, h, handle.max_offset().x, at), handle.offset().y));
            } else {
                handle.set_offset(point(handle.offset().x, offset_for(track, h, handle.max_offset().y, at)));
            }
        };
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            if !phase.bubble() || e.button != MouseButton::Left || !hb.is_hovered(window) {
                return;
            }
            let y = pick(e.position);
            // On the thumb: held where it was taken; on the track: the thumb comes there.
            let grab = if y >= top && y <= top + h { y - top } else { h / 2. };
            if !(y >= top && y <= top + h) {
                set(&handle, y - grab);
            }
            down.0.set(Some(grab));
            if across {
                cx.stop_propagation();
            }
            window.refresh();
        });
        let (handle, moving) = (self.handle.clone(), drag.clone());
        window.on_mouse_event(move |e: &MouseMoveEvent, _, window, _| {
            let Some(grab) = moving.0.get() else { return };
            if e.pressed_button != Some(MouseButton::Left) {
                moving.0.set(None);
                return;
            }
            set(&handle, pick(e.position) - grab);
            window.refresh();
        });
        window.on_mouse_event(move |_: &MouseUpEvent, _, window, _| {
            if drag.0.take().is_some() {
                window.refresh();
            }
        });
    }
}
