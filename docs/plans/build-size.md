# The build folder stays small

Asked 2026-11-09 (user): `target/` had grown to 435 GB in three days (`cargo clean` removed
452.1 GiB, 179 595 files). Fix it the way large Rust projects do, not with patches. No
requirement id and no milestone (hence the file name): this is how devdeck is built and
tested, not what it does. Touches no OS code (R11.6) and no view: there are no states to
check on screen.

## Causes (measured 2026-11-09, before the clean)

- **One binary per test file.** `crates/devdeck-app/tests/` has 31 files, `devdeck-ui/tests/`
  6. Cargo links each into its own executable carrying GPUI and the rest with full debug
  info: about 1.1 GB each, so one build of the app's tests was about 35 GB. `debug/deps`
  was 372 GB, `debug/incremental` 55 GB (one incremental folder per test crate).
- **Several full builds side by side.** Which packages are selected changes the features
  of shared dependencies: with `--workspace`, `tokio` (from `devdeck-kube`'s
  dev-dependencies), `http-body-util` (same) and `schemars` (`chrono04`, from `rmcp` in
  `devdeck-mcp`) get features that `-p devdeck-app` does not give them, and `schemars` is
  a dependency of GPUI, so GPUI and everything above it get new hashes. `-p devdeck-app`
  (WORKFLOW "while building"), `scripts/test_changed.py` (a different `-p` set per change)
  and `--workspace` (before a commit) each kept a complete set of binaries.
- **Old sets are never removed.** A new hash is a new file; Cargo deletes nothing. Hashes
  also change with `Cargo.lock` and with a new rustc (9 commits changed `Cargo.toml` or
  `Cargo.lock` in those three days), which fits the 22 copies of the `settings` test
  binary. That last link is inferred from the file dates, not proven.

- **Memory too.** systemd-oomd killed the Claude app, and every build and test under it,
  twice on 2026-11-09: at 20:15 (a session's `cargo test`) and at 22:12 (Step 0 below,
  48 s into `cargo test --workspace` after a one-line edit in the app: every app test
  binary relinked at once on 20 cores, then the whole suite), the app's group at 53.4 GB.
  Which of the two phases took the memory is not known.

Level: how the workspace is laid out and built (Cargo profile, test layout, the commands
in the workflow). Cargo's own answers are unstable on 1.99 (`-Z gc`,
`-Z feature-unification`), so they are not used.

## The rule (what large projects do)

1. **One integration test binary per crate** (Cargo's own `tests/testsuite/`; matklad,
   "Delete Cargo Integration Tests").
2. **A dev profile whose debug info is limited and split out of the binaries**, and a
   separate profile with full debug info for a debugger session (Zed's `Cargo.toml`, the
   project GPUI comes from).
3. **One way to build**: from the workspace root, the whole workspace every time; tests
   are chosen by name, never by `-p` or by running cargo in a member's folder.
4. **`target/` is a cache**, cleared at fixed moments, written down where a newcomer looks.

## Steady state

Some builds differ on purpose and each keeps one set, rebuilt in place by the same
command: check/clippy (`.rmeta` only), tests (dev-dependency features: GPUI with
`test-support`), the debug app (`cargo build`, `cargo run`), the snapshot tool
(`--features devdeck-app/snapshot`), release. What must not happen is a second set for the
same kind of build. The proof is Cargo's own: running a command again after an edit
compiles only workspace crates, never a dependency (no `Compiling <dependency>` line).

## Before starting

The working tree has uncommitted work (`CLAUDE.md`, `deck.rs`, `window_view.rs`,
`tests/deck.rs`, `layout/`, `REQUIREMENTS.md`, `WORKFLOW.md`, `PLATFORMS.md`). This plan
moves `tests/deck.rs` and edits `WORKFLOW.md`, `PLATFORMS.md` and `CLAUDE.md`, so it starts
on a clean tree: that work is committed (or set aside) first, by its owner. Nothing here
is mixed into it.

Every number below is recorded with `cargo --version`. Cargo 1.100 (due 2026-11-12) moves
intermediate build files to a new layout, so the baseline and the results are taken on
the same toolchain, and sizes are `du -sh target` totals plus the number of test
executables (from `cargo test --workspace --no-run`), never paths inside `target/`.

## Steps (one commit each)

### Step 0: baseline (no commit)

From a clean `target/`, with the current layout:
- `cargo test --workspace --no-run`: time, `du -sh target`, test executables;
- then `cargo test -p devdeck-app --no-run`, then `cargo build`: `du -sh target` after
  each (what a second selection and the app build add);
- the step loop as WORKFLOW measures it: a one-line edit in `crates/devdeck-app/src/deck.rs`,
  then build the tests and run them; a one-line edit in a test file, same;
- one dependency bump: `cargo update -p <a small leaf crate>`, `cargo test --workspace
  --no-run`, `du -sh target`, then `git checkout Cargo.lock` (what one `Cargo.lock` change
  leaves behind).

`cargo clean` between measurements: `target/` holds only build output (checked
2026-11-09: nothing outside it points in, nothing in it is tracked).

Measured 2026-11-09, cargo 1.99.0, before the run was killed (see "Memory too"):

| From a clean `target/` | Time | `target/` after |
|---|---|---|
| `cargo test --workspace --no-run` (50 test executables) | 130 s | 39 GB |
| then `cargo test -p devdeck-app --no-run` | 136 s | 69 GB |
| then `cargo build` | 73 s | 72 GB |
| then `cargo clippy --workspace --all-targets` | 37 s | 74 GB |
| one-line edit in `src/deck.rs`, `cargo test -p devdeck-app --test deck` | 3.5 s + 0.35 s run | |

The second selection alone rebuilt 30 GB. The rest of Step 0 (the whole suite after an
edit, the dependency bump) is not run on the old layout: it is what was killed.

**Every measurement from here on runs in its own capped scope**: `systemd-run --user
--scope -p MemoryMax=32G -p MemorySwapMax=0 …`, sampling the scope's peak anonymous
memory. Going over kills that build only, never the Claude app. Peak memory is recorded
next to every time and size.

### Step 1: the dev profile (`Cargo.toml`)

```toml
[profile.dev]
# Debug info stays in `.dwo` files next to the objects instead of being copied into every
# binary; `limited` keeps file and line for backtraces, not variables.
split-debuginfo = "unpacked"
debug = "limited"

# A debugger session with variables: `cargo build --profile dbg` (its own folder,
# `target/dbg`; delete it when done).
[profile.dbg]
inherits = "dev"
debug = "full"
```

No `build-override` table: Cargo already builds build scripts and proc-macros without
debug info and shares crates used both at build and at run time (Zed needs the table only
because it changes `codegen-units`). The existing `[profile.dev.package]` lines (PNG
crates at `opt-level = 3`) stay. Release is unchanged.

Binaries with no unit tests stop being built as test executables: `test = false` on the
`devdeck` `[[bin]]` (`crates/devdeck-app/Cargo.toml`, links all of GPUI) and on
`devdeck-mcp`'s. `devdeck-ctl/src/main.rs` has tests: unchanged.

What it costs: a debugger in a dev build shows the call stack, not local variables (use
`dbg`); a debug binary copied out of `target/` has no debug info.

Check: Step 0's measurements again; a failing test under `RUST_BACKTRACE=1` still shows
`file:line` frames (a temporary `panic!` in one test, not committed).

Measured 2026-11-09, cargo 1.99.0, capped scope, test layout unchanged:

| | Before | After Step 1 |
|---|---|---|
| cold `cargo test --workspace --no-run` | 130 s, 39 GB, 50 executables | 73 s, 14 GB, 48 executables, peak 7.8 GB |
| one-line app edit, `cargo test --workspace` (build + run) | killed at 53.4 GB | 32 s, peak 8.5 GB, 777 passed |
| the whole suite again, nothing changed | | 24 s, peak 0.2 GB |
| then `cargo test -p devdeck-app --no-run` | +30 GB | +10 GB (27 crates rebuilt: Step 3's cause) |

A panic in a test still names `tests/deck.rs:413:14` in its backtrace.

### Step 2: one test binary per crate

- `devdeck-app`: `git mv tests/*.rs tests/app/`, and a new `tests/app/main.rs` that only
  declares the 31 modules (`mod actions; mod api; …`), in alphabetical order. Test target
  `app`.
- `devdeck-ui`: `git mv` the 6 files and `common/` to `tests/ui/`; `tests/ui/main.rs`
  declares `mod common;` once and the 6 modules. Each file's `mod common;` becomes
  `use crate::common;` (one line for one, so line numbers quoted elsewhere stay right).
  Target `ui`.
- `devdeck-core`, `devdeck-ctl`, `devdeck-mcp` have one test file each: unchanged.
  `devdeck-kube/tests/` holds only fixtures.
- References that follow the move: `docs/WORKFLOW.md` ("Components are tested on their
  own": `crates/devdeck-ui/tests/`, `tests/common`), `docs/PLATFORMS.md:29`
  (`devdeck-ui/tests/code.rs:184`) and `:31` (`devdeck-app/tests/lifecycle.rs`,
  `tests/pty_driver.rs`), `crates/devdeck-ui/src/code.rs:160` (`tests/code.rs`),
  `crates/devdeck-ui/Cargo.toml:16` (`tests/common`). `docs/perf/` and `docs/plans/` are
  history: unchanged.

One process instead of 31: until now Cargo ran the test binaries one after another, so
tests of different files never ran at the same time; now all 259 app tests share one
process and run in parallel. Checked: no test file has crate-level attributes, statics or
process-wide changes (`set_var`, `set_current_dir`); ports are bound on `:0`; folders come
from `test_dir` / `TempDir` (per-process counter); the app's own globals are not asserted
on; `lifecycle::install` (the signal handler) is called by no test. One class remains:
children spawned through `pre_exec` take the fork path and briefly hold copies of other
tests' descriptors (`lifecycle.rs:24-30` already handles it for the lock; `ports.rs:209-210`
expects a port to be free right after `drop`). A test that fails for this reason is fixed
where its assumption is, the way `lifecycle.rs` does it, in its own commit; tests are never
made to run one at a time.

Check: the same number of tests pass as before the move (259 app, 68 ui, from the test
output); the whole suite three times in a row; Step 0's measurements again, including the
edit of one test file (now the whole `app` test crate recompiles: the cost to watch).

Measured 2026-11-09, cargo 1.99.0, capped scope (`api_waits.rs`, committed meanwhile, makes
32 app files):

| | After Step 1 | After Step 2 |
|---|---|---|
| cold `cargo test --workspace --no-run` | 73 s, 14 GB, 48 executables, peak 7.8 GB | 69 s, 6.2 GB, 12 executables, peak 4.9 GB |
| one-line app edit, `cargo test --workspace` | 32 s, peak 8.5 GB | 9.6 s, peak 0.9 GB |
| the whole suite again, nothing changed | 24 s | 12 s, peak 0.3 GB |
| one-line edit in a test file, rebuild | 0.9 s | 1.9 s |

777 tests pass, as before the move. The first run had two failures in `ports.rs`, of the
fork class above: a dropped listener still held by another test's child for a moment
(`a_start_is_refused…` after `drop(held)`, `stop_holder_on_our_own_run…` right after the
stop). Both now wait for the port to be free, as `lifecycle.rs` does for its lock
(`wait_free`); five runs in a row then passed. The fix is in the Step 2 commit rather than
its own: it only fails with the move, and every commit passes the suite.

### Step 3: one way to build (commands, rules, docs)

- A rule in `CLAUDE.md` "Rules that are easy to break", and in `WORKFLOW.md`: cargo runs
  from the workspace root with `--workspace` (`--all-targets` for check and clippy); tests
  are chosen by name; never `-p`, never from a member's folder. A new test file is a module
  of `tests/<crate>/main.rs`, never a new top-level file in `tests/` (that is a new binary
  again; a file with no `mod` line is never compiled).
- Before a commit (`CLAUDE.md`, `WORKFLOW.md` §6, `BUILDING.md`, `PLATFORMS.md:176,180-181`):
  `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`
  (clippy without `--all-targets` resolves the build's features, another set).
- While building (`WORKFLOW.md` "Keep the step loop fast", and "Parallel builders", which
  today says only "`cargo check` and their own tests"): `cargo check --workspace
  --all-targets`, and one area's tests by name, `cargo test --workspace --test app deck::`
  (asking for a test target resolves the same features as `cargo test --workspace`). The
  docs say that a name is a substring (`sidebar::` also runs `notes_sidebar::`), and that
  a crate's unit tests are `--lib`, not `--test app`. The step loop's 2026-11-07 numbers
  are replaced by the new ones.
- `scripts/test_changed.py` selects crates with `-p`: removed, with `WORKFLOW.md:44`, if
  the whole suite after a one-file edit in the app stays close to today's loop (4.4 s
  build + 6 s run); `WORKFLOW.md:47` ("the whole suite" only at the milestone's end) goes
  with it. If it does not stay close, the numbers go to the user before anything else is
  changed.
- `BUILDING.md:22`: `cargo run --release --bin devdeck` from the root (the selection
  `install.sh` uses, so the two share one release set); the snapshot tool: `cargo run --bin
  devdeck-snapshot --features devdeck-app/snapshot`. `PLATFORMS.md`'s `cargo check -p
  devdeck-sys --target aarch64-apple-darwin` becomes `--workspace`; another target already
  has its own folder.

Check: every `cargo (test|check|clippy|build|run)` line in `CLAUDE.md`, `README.md`,
`docs/*.md` and `scripts/` read and matched to the rule (plans and perf are history);
each kind of build from "Steady state" run twice with an edit between: the second run
compiles no dependency.

### Step 4: `target/` is a cache (`docs/BUILDING.md`, `WORKFLOW.md`)

In `BUILDING.md`: what `target/` holds, the sets of "Steady state" and their measured
sizes, that a `Cargo.lock` change or a new rustc leaves the old set behind (measured in
Step 0), and `--profile dbg` with `target/dbg` deleted after. In `WORKFLOW.md`: `cargo
clean` at the end of every milestone (it already ends with a full suite and release
builds) and after `rustup update`; installed binaries are copies (`install.sh`), so
nothing breaks. Can go in the Step 3 commit.

## Done when

- One build of the workspace with its tests is measured and the number is in
  `BUILDING.md`, with the baseline next to it and `cargo --version`.
- Each kind of build, repeated after an edit, compiles no dependency.
- The peak memory of a cold `cargo test --workspace --no-run` and of `cargo test
  --workspace` after an app edit is measured, recorded in `BUILDING.md`, and leaves the
  machine usable (well under its 60 GB, with the desktop and the Claude app running).
- The step loop is measured again in `WORKFLOW.md`, and is not slower than the baseline
  by more than the user accepts.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`
  pass.

## Out of scope

- `cargo-nextest`, `cargo-sweep`, `cargo-hakari` (a workspace-hack crate), `sccache`,
  another linker: each needs installing, and nothing is installed unasked.
- `-Z gc`, `-Z feature-unification`: nightly only.
- Pinning the toolchain (`rust-toolchain.toml`): it would make "a new rustc" a chosen
  moment, but it is a decision of its own, for the user.
- What the editor's rust-analyzer builds: its own settings, outside the repo.
- The release profile, `install.sh`, the snapshot tool's code, test contents (only their
  place and `mod` lines change), except a test fixed for the one-process reason above.

## Review (2026-11-09, fresh subagent) and assessment

1. `cargo build` and `cargo test --workspace` are always two sets (dev-dependency
   features), so "no growth" could not pass: **accepted**; "Steady state" lists the sets
   and the criterion became "a repeat compiles no dependency".
2. The `build-override` table's reason was wrong (Cargo already shares those crates; Zed
   needs it for `codegen-units`): **accepted**, the table is gone.
3. Cargo 1.99 has a built-in `debug` profile; use it instead of `dbg`: **rejected**; a
   custom `dbg` works the same and its folder (`target/dbg`) is documented, the built-in
   one's folder is not verified.
4. Release was two sets (`BUILDING.md`'s `-p devdeck-app` vs `install.sh`'s selection):
   **accepted**, `BUILDING.md` runs from the root.
5. Commands that drift were not all covered (clippy without `--all-targets`, running in a
   member's folder, "Parallel builders"), and nothing tells agents: **accepted**, a rule in
   `CLAUDE.md` and `WORKFLOW.md`, and every cargo line is read.
6. Path references the move breaks: **accepted**, listed in Step 2.
7. The one-binary layout was not written down for future tests: **accepted**, in the rule.
8. Descriptors inherited across `fork` between tests now running together: **accepted**,
   named in Step 2 with how a failing test is fixed (never by serializing).
9. The baseline skipped the biggest cause (a `Cargo.lock` change, a new rustc), Cargo 1.100
   changes the layout, and "now and then" is not a trigger: **accepted**; a dependency
   bump is measured, numbers carry `cargo --version` and are totals, `cargo clean` has
   fixed moments. Pinning the toolchain is left to the user.
10. `du` is coarse; use "no `Compiling` line for a dependency": **accepted**, it is the
    proof now.
11. `test = false` on binaries without unit tests: **accepted** (checked: `devdeck` and
    `devdeck-mcp`'s `main.rs` have no tests, `devdeck-ctl`'s has).
12. Test names are substrings, and `--test app` skips unit tests: **accepted**, both in the
    docs.
13. `use crate::common;` is correct; one line is simpler: **rejected** for the simpler
    form, the one-for-one line keeps the line numbers `PLATFORMS.md` quotes.
14. `WORKFLOW.md:44,47` and the 2026-11-07 numbers contradict the rule: **accepted**,
    named in Step 3.
15. The fallback for `--workspace --test app` is not needed: **accepted**, removed; Step 3
    runs it.
