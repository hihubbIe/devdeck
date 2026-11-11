//! The component library's tests, one binary: a new test file is a module here, never a
//! new file in `tests/` (docs/WORKFLOW.md). One area: `cargo nextest run --workspace input::`.

mod common;

mod bars;
mod code;
mod components;
mod host;
mod input;
mod overlay;
mod scroll;
mod toolbar;
