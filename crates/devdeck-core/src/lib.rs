//! devdeck logic that has no UI. Everything here is plain data and pure functions so it
//! can be unit tested without a GPU.

pub mod apps;
pub mod catalog;
pub mod cluster;
pub mod config;
pub mod demo;
pub mod discovery;
pub mod environment;
pub mod files;
pub mod fuzzy;
pub mod git;
pub mod layout;
pub mod layout_book;
pub mod links;
pub mod list;
pub mod notes;
pub mod ports;
pub mod process;
pub mod recent_args;
pub mod runs;
pub mod scope;
pub mod settings;
pub mod state;
pub mod terminal;
pub mod testing;
pub mod theme_file;
pub mod toolchains;
pub mod view_addr;

/// FNV-1a: a hash stable across runs and platforms (demo data, file fingerprints).
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}
