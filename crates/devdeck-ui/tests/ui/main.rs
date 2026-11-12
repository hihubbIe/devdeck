//! The component library's tests, one binary: a new test file is a module here, never a
//! new file in `tests/` (docs/WORKFLOW.md). One area: `cargo nextest run --workspace input::`.

mod common;

mod bars;
mod chooser;
mod code;
mod components;
mod host;
mod input;
mod nav;
mod overlay;
mod picker;
mod scroll;
mod toolbar;
