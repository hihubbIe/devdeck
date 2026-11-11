//! devdeck's theme and shared components (R9.2, R9.3). This is the only crate allowed to
//! use raw `px`/`rems` (see `clippy.toml`); everything else takes sizes from [`Theme`].
#![allow(clippy::disallowed_methods)]

pub mod bars;
pub mod code;
pub mod components;
pub mod icons;
pub mod overlay;
pub mod picker;
pub mod scroll;
pub mod table;
pub mod terminal;
pub mod text_buffer;
pub mod text_input;
pub mod theme;
pub mod toolbar;

pub use bars::{PodMark, pod_square, pods_bar, usage_bar};
pub use code::{Code, CodeEvent, MenuOpener, Rendered};
pub use components::*;
pub use icons::{APP_ICON, APP_NAME, Assets, Icon, icon, spinner};
pub use picker::{OnPick, PickGroup, PickOption, Picker, PickerSpec, open_picker};
pub use scroll::{Scrolls, scroll_list};
pub use table::{Column, table_head, table_more, table_row};
pub use overlay::{MenuItem, ModalFocus, NewToast, Note, NoteAction, ToastKind, modal_frame, open_menu, pop_modal, push_modal, render_layer, toasts};
pub use text_input::{InputEvent, TextInput};
pub use toolbar::{Count, Fold, MenuValue, ToolAction, ToolItem, Toolbar, ToolbarState};

pub use theme::{Theme, apply_window_scale, logical, theme};
