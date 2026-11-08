//! devdeck's theme and shared components (R9.2, R9.3). This is the only crate allowed to
//! use raw `px`/`rems` (see `clippy.toml`); everything else takes sizes from [`Theme`].
#![allow(clippy::disallowed_methods)]

pub mod code;
pub mod components;
pub mod icons;
pub mod overlay;
pub mod scroll;
pub mod terminal;
pub mod text_buffer;
pub mod text_input;
pub mod theme;

pub use code::{Code, CodeEvent, markdown_view};
pub use components::*;
pub use icons::{APP_ICON, APP_NAME, Assets, Icon, icon, spinner};
pub use scroll::{Scrolls, scroll_list};
pub use overlay::{MenuItem, ModalFocus, ToastKind, modal_frame, open_menu, pop_modal, push_modal, render_layer, toasts};
pub use text_input::{InputEvent, TextInput};

pub use theme::{Theme, apply_window_scale, logical, theme};
