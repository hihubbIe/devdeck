# How we work on devdeck

## A step, start to finish

1. **Plan.** Write the step's plan in `docs/plans/<milestone>-<step>.md`: goal, the
   requirement ids it covers, files touched, the states to check visually, the tests
   to add, what is out of scope, and whether it touches the OS (R11.6). If it does, the
   plan names the `devdeck-sys` capability and, per OS, a backend or "unsupported" with
   its reason (`docs/PLATFORMS.md` §4).
2. **Review.** A subagent reviews the plan against `REQUIREMENTS.md`, `DESIGN.md` and
   `PLATFORMS.md` with fresh eyes: gaps, wrong assumptions, missing states, test holes,
   OS-specific code outside `devdeck-sys`.
3. **Assess.** Accept or reject each review point, with a one-line reason, appended to
   the plan file.
4. **Build.** Implement. Tests come with the code, not after.
5. **Check visually.** Run the app and look at every state listed in the plan (empty,
   loading, error, missing tool, protected env, collapsed, detached, narrow pane…).
   Fix what doesn't match the design before moving on.
6. **Test.** `cargo test --workspace` must pass. `cargo clippy --workspace --all-targets
   -- -D warnings` must pass. New features ship with unit tests; bugs ship with a
   regression test.
7. **Commit.** One commit per step, message naming the step and requirement ids.
   Update the requirement checkboxes in `REQUIREMENTS.md` in the same commit.

## Fixing problems

- Before changing code, write down the root cause and the level it belongs to: a shared
  component or the view engine, how we use GPUI, or the architecture. Fix it there, once,
  generically; several targeted patches for one cause mean the fix is at the wrong level.
- The engine (layout, deck, registry, components) and the views built on it are designed
  together: a view-specific workaround usually means the engine is missing something.
- A subagent reviews the fix plan and then the diff with fresh eyes, given the code and the
  symptom rather than the proposed conclusion.

## Keep the step loop fast

Measured 2026-11-09 (cargo 1.99), medians of five, after a one-line edit in
`devdeck-app`: clippy 2.0 s, one area's tests 3.4 s, the whole suite (777 tests) 15.0 s,
of which about 12 s is running the tests; peak memory under 1 GB. The test binaries run
one after another, each as long as its slowest test or its share of the work: the app's
6.4 s, `devdeck-core`'s unit tests 2.2 s and its `tests/git.rs` 2.3 s (tests that wait on
real processes and hooks), the rest under 1 s. Rendering every snapshot takes
45 s (it was 133 s before the PNG crates were optimized in debug builds, see
`Cargo.toml`); two snapshots take about 3 s.

**One way to build.** Cargo runs from the workspace root with `--workspace`
(`--all-targets` for check and clippy) and tests are chosen by name. Never `-p`, never from
a crate's folder: which packages are selected changes the features of shared dependencies
(GPUI among them), so each selection is a full build of its own, kept next to the others
in `target/` (`docs/BUILDING.md`). Each crate has one integration test binary: a new test
file is a module of `tests/<crate>/main.rs` (`tests/app/`, `tests/ui/`), never a new file
in `tests/` (a new binary) and never a file without its `mod` line (never compiled). No
crate runs doctests (`doctest = false` in each `[lib]`, 0.5 s off every whole suite): an
example in a doc comment is not tested, so a test goes in a test file.

- **While building:** the tests of the area being changed, by name: `cargo test
  --workspace --test app deck::` (a name is a substring: `sidebar::` also runs
  `notes_sidebar::`); a crate's unit tests with `--lib`: `cargo test --workspace --lib
  layout`. For errors without tests, clippy (`cargo clippy --workspace --all-targets`),
  not `cargo check`: the two keep separate results for our crates, so running both
  repeats the work, and a test build already reports every compile error except the
  binary's (`crates/devdeck/src/main.rs`, which only clippy and `cargo build` compile):
  after changing an API it uses, run clippy.
- **Before the commit:** the whole suite and clippy (step 6), and the snapshots of the
  states the step adds or touches (the snapshot tool takes a name filter:
  `-- <out-dir> 01-,20-`).
- **End of the milestone only:** every snapshot, release builds, measurements, stress and
  leak runs, then `cargo clean` (see "A milestone").
- **Reviews:** the plan is reviewed before any code, by a reviewer who reads the GPUI source
  where the behaviour depends on it; then one review of the diff. Reviewers run in the
  same turn as the build, so one approval covers the step: build, review, fixing what the
  review finds within the step's scope, commit. Findings outside the scope wait for the
  user.
- **Parallel builders:** when a step splits into modules on separate files, the plan fixes
  their interfaces first; then one subagent builds each module with its tests, in this
  checkout (a worktree would need its own cold GPUI build), never two on the same file.
  Builders run their own tests by name and clippy (never `-p`); the lead runs the
  combined build, the review and the commit. Builds share `target/`'s lock: they queue.

## Components are tested on their own

`devdeck-ui` is the component library every view draws with (R9.3). Each component has
tests in `crates/devdeck-ui/tests/ui/`, in a host window (`tests/ui/common`) with nothing
of the app: every interaction by mouse (click, hover, drag, wheel) and by keyboard,
disabled states, focus. A new component or a change to one comes with its tests. The
app's own tests cover only how components fit in it (menus over panes, modals blocking
app keys).

## Tests leave nothing behind

A test that needs a folder takes it from `devdeck_core::testing::TempDir` (core tests) or
`devdeck_app::test_dir` (app tests: the folder lives as long as the test's app). Folders
sit under `/tmp/devdeck-tests/<pid>/`, are removed when dropped, and a later test run
removes those of runs that crashed. Tests never touch real folders.

## A milestone, start to finish

- A milestone is a series of steps (see `ROADMAP.md`).
- Before it closes: run `scripts/measure.sh` on a release build and record the numbers
  in `docs/perf/<milestone>.md` (RSS and CPU idle and under load, latency, binary size,
  leak test). Compare with the previous milestone; investigate any growth.
- Then `cargo clean`, and after every `rustup update`: a dependency change or a new rustc
  leaves the previous build behind in `target/` (`docs/BUILDING.md`, "The build folder").
- At the end of every milestone, work stops and the user tests the app by hand.
  Feedback goes into `REQUIREMENTS.md` / `DESIGN.md` before the next milestone starts.

## Profiling

- **CPU** (where time goes): `perf`. Build with symbols into a separate folder:
  `CARGO_PROFILE_RELEASE_DEBUG=true CARGO_PROFILE_RELEASE_STRIP=false cargo build --release --target-dir target/profiling`,
  then `perf record -F 2000 -g -o perf.data <bin>` during the scenario, and
  `perf report -i perf.data --stdio --no-children --sort symbol --percent-limit 1`
  (then `--sort srcline`, or `-g caller --symbol-filter=<name>` for callers). Needs
  `kernel.perf_event_paranoid` ≤ 1.
- **Memory** (growth, leaks): `heaptrack -o <file> <bin>` on the same build, under the
  leak scenario with temporary `DEVDECK_STATE_DIR` / `DEVDECK_CONFIG_DIR`; read it with
  `heaptrack_print` (leaks at exit, peak, call sites that grow).
- **Leak runs stay short**: every stress cycle does one of everything (each view type, a run,
  overlays, a detached window, a workspace and a layout switch), so a heaptrack diff of a
  5-cycle and a 25-cycle run shows the per-cycle cost in about a minute. The longer USS run
  stops at 1 000 cycles if 500–1 000 still grows, and only confirms a flat curve otherwise.
- Delete recordings (`perf.data`, heaptrack files, `target/profiling`) once read.

## Definition of done for a feature

- Works with mouse and with keyboard.
- Every state has been seen on screen: empty, normal, long text (truncates, never
  wraps), error, unavailable with a reason.
- Uses only shared components and theme tokens.
- Touches the OS only through `devdeck-sys`, with each OS covered by a backend or an
  "unsupported" reason, and a conformance test for any new capability (R11.6).
- Unit tested; the whole suite passes.
- Documented where a newcomer would look (view guide, config reference…).
