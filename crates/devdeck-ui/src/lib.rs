//! devdeck's theme and shared components (R9.2, R9.3). This is the only crate allowed to
//! use raw `px`/`rems` (see `clippy.toml`); everything else takes sizes from [`Theme`].
#![allow(clippy::disallowed_methods)]

pub mod components;
pub mod overlay;
pub mod text_buffer;
pub mod text_input;
pub mod theme;

pub use components::*;
pub use overlay::{MenuItem, ModalFocus, ToastKind, modal_frame, open_menu, pop_modal, push_modal, render_layer, toasts};
pub use text_input::{InputEvent, TextInput};

/// Key bindings of the shared components (text input, menus, modals).
pub fn bind_keys(cx: &mut gpui::App) {
    text_input::bind_keys(cx);
    overlay::bind_keys(cx);
}
pub use theme::{Theme, apply_window_scale, logical, theme};
