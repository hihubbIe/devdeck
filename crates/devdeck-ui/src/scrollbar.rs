//! A scrollbar over a scroll container's right edge (R9.6): a thumb whose height says how
//! much of the content shows and whose place says where; nothing when everything shows.
//! It reads the container's [`ScrollHandle`] as it paints (a wheel scroll redraws the
//! view), so it never lags. Dragging the thumb scrolls; a press on the track brings the
//! thumb there, then drags. It sits over the content (absolute, a fixed width): showing
//! or hiding it moves nothing (R9.10).
//!
//! Put it right after the container, in a `relative` parent: [`with_scrollbar`] does.

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
    Scrollbar { id: id.into(), handle: handle.clone() }
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
        style.inset.top = Pixels::ZERO.into();
        style.inset.bottom = Pixels::ZERO.into();
        style.inset.right = Pixels::ZERO.into();
        style.size.width = t.size(10.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, _: &mut App) -> Option<Hitbox> {
        (self.handle.max_offset().y > Pixels::ZERO).then(|| window.insert_hitbox(bounds, HitboxBehavior::Normal))
    }

    fn paint(&mut self, id: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), hitbox: &mut Option<Hitbox>, window: &mut Window, cx: &mut App) {
        let Some(hitbox) = hitbox.clone() else { return };
        let t = theme(cx).clone();
        let (viewport, max, offset) = (self.handle.bounds().size.height, self.handle.max_offset().y, self.handle.offset().y);
        let pad = t.size(2.);
        let track = Bounds { origin: point(bounds.left(), bounds.top() + pad), size: size(bounds.size.width, bounds.size.height - pad * 2.) };
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
        let width = if hovered { t.size(7.) } else { t.size(5.) };
        let shape = Bounds { origin: point(bounds.right() - width - pad, top), size: size(width, h) };
        let color = if hovered { t.mute.opacity(0.6) } else { t.scrollbar };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(shape, color).corner_radii(Corners::all(width / 2.)));
        });
        window.set_cursor_style(CursorStyle::Arrow, &hitbox);

        let handle = self.handle.clone();
        let (down, hb) = (drag.clone(), hitbox.clone());
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, _| {
            if !phase.bubble() || e.button != MouseButton::Left || !hb.is_hovered(window) {
                return;
            }
            let y = e.position.y;
            // On the thumb: held where it was taken; on the track: the thumb comes there.
            let grab = if y >= top && y <= top + h { y - top } else { h / 2. };
            if !(y >= top && y <= top + h) {
                handle.set_offset(point(handle.offset().x, offset_for(track, h, handle.max_offset().y, y - grab)));
            }
            down.0.set(Some(grab));
            window.refresh();
        });
        let (handle, moving) = (self.handle.clone(), drag.clone());
        window.on_mouse_event(move |e: &MouseMoveEvent, _, window, _| {
            let Some(grab) = moving.0.get() else { return };
            if e.pressed_button != Some(MouseButton::Left) {
                moving.0.set(None);
                return;
            }
            handle.set_offset(point(handle.offset().x, offset_for(track, h, handle.max_offset().y, e.position.y - grab)));
            window.refresh();
        });
        window.on_mouse_event(move |_: &MouseUpEvent, _, window, _| {
            if drag.0.take().is_some() {
                window.refresh();
            }
        });
    }
}
