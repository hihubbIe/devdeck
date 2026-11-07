# devdeck

Native Rust desktop cockpit to find, run and deploy apps. Read these before any change:

- `docs/REQUIREMENTS.md` what must be true (ids R1.1…)
- `docs/DESIGN.md` look, architecture, layout model, decisions
- `docs/WORKFLOW.md` how a step is planned, reviewed, built, checked, tested, committed
- `docs/ROADMAP.md` milestones; stop after each one for the user's manual test

Rules that are easy to break:
- Text never wraps unless a view opts in; truncate with an ellipsis instead (R4.4).
- Every list is alphabetical, Pinned + All (R4.1, R4.2).
- Views use `devdeck-ui` components and theme tokens only (R9.3).
- Every user operation is an action usable by mouse and keyboard (R5.1).
- Nothing machine-specific in code; it belongs in config (R1.5).
- No git or editor features (R6.10).
- `cargo test --workspace` and `cargo clippy --workspace -- -D warnings` pass before every commit.
