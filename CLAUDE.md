# devdeck

Native Rust desktop cockpit to find, run and deploy apps. Read these before any change:

- `docs/REQUIREMENTS.md` what must be true (ids R1.1…)
- `docs/DESIGN.md` look, architecture, layout model, decisions
- `docs/WORKFLOW.md` how a step is planned, reviewed, built, checked, tested, committed
- `docs/ROADMAP.md` milestones; stop after each one for the user's manual test
- `docs/PLATFORMS.md` what is OS-specific, and the OS layer every feature goes through

Rules that are easy to break:
- Text never wraps unless a view opts in; truncate with an ellipsis instead (R4.4).
- Every list is alphabetical, Pinned + All (R4.1, R4.2).
- Views use `devdeck-ui` components and theme tokens only (R9.3).
- Every user operation is an action usable by mouse and keyboard (R5.1).
- Nothing machine-specific in code; it belongs in config (R1.5).
- Files are view only; notes are the one thing devdeck edits (R12).
- OS-specific code only in `devdeck-sys`, a backend or "unsupported" per OS (R11.6).
- After touching `devdeck-sys`, `devdeck-core`, `devdeck-api`, `devdeck-ctl` or
  `devdeck-mcp`, the Mac compile check passes too (`docs/PLATFORMS.md` §4): `cargo clippy
  --target aarch64-apple-darwin -p devdeck-sys -p devdeck-core -p devdeck-api -p
  devdeck-ctl -p devdeck-mcp --all-targets -- -D warnings` (the one place `-p` is right:
  the other crates can't build for a Mac from Linux). The macOS work is paused:
  `docs/plans/macos.md`, "Where it stands".
- `cargo nextest run --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` pass before every commit.
- Cargo runs from the root with `--workspace`, tests chosen by name, never `-p` or from a
  crate's folder: another selection is another full build. A new test file is a module of
  `tests/<crate>/main.rs`, never a new file in `tests/` (`docs/WORKFLOW.md`).
