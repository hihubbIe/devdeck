//! devdeck's theme and shared components (R9.2, R9.3). This is the only crate allowed to
//! use raw `px`/`rems` (see `clippy.toml`); everything else takes sizes from [`Theme`].
#![allow(clippy::disallowed_methods)]

pub mod components;
pub mod theme;

pub use components::*;
pub use theme::{Theme, apply_window_scale, logical, theme};
