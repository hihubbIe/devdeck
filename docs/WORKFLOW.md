# How we work on devdeck

## A step, start to finish

1. **Plan.** Write the step's plan in `docs/plans/<milestone>-<step>.md`: goal, the
   requirement ids it covers, files touched, the states to check visually, the tests
   to add, what is out of scope.
2. **Review.** A subagent reviews the plan against `REQUIREMENTS.md` and `DESIGN.md`
   with fresh eyes: gaps, wrong assumptions, missing states, test holes.
3. **Assess.** Accept or reject each review point, with a one-line reason, appended to
   the plan file.
4. **Build.** Implement. Tests come with the code, not after.
5. **Check visually.** Run the app and look at every state listed in the plan (empty,
   loading, error, missing tool, protected env, collapsed, detached, narrow pane…).
   Fix what doesn't match the design before moving on.
6. **Test.** `cargo test --workspace` must pass. `cargo clippy --workspace -- -D warnings`
   must pass. New features ship with unit tests; bugs ship with a regression test.
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

Measured 2026-11-07 after a one-file edit in `devdeck-app`: `cargo check -p devdeck-app`
0.8 s, clippy on the workspace 1.1 s, building the tests 4.4 s, running them 6 s.
Linking is not a cost (rustc links with lld). Rendering every snapshot takes 45 s (it
was 133 s before the PNG crates were optimized in debug builds, see `Cargo.toml`); two
snapshots take about 3 s.

- **While building:** `cargo check -p <crate>` and the test file of the area being changed
  (`cargo test -p devdeck-app --test deck`).
- **Before the commit:** clippy and the whole suite once (about 10 s), and the snapshots of
  the states the step adds or touches (the snapshot tool takes a name filter:
  `-- <out-dir> 01-,20-`).
- **End of the milestone only:** every snapshot, release builds, measurements, stress and
  leak runs.
- **Reviews:** the plan is reviewed before any code, by a reviewer who reads the GPUI source
  where the behaviour depends on it; then one review of the diff. Reviewers run in the
  same turn as the build, so one approval covers the step: build, review, fixing what the
  review finds within the step's scope, commit. Findings outside the scope wait for the
  user.
- **Parallel builders:** when a step splits into modules on separate files, the plan fixes
  their interfaces first; then one subagent builds each module with its tests, in this
  checkout (a worktree would need its own cold GPUI build), never two on the same file.
  Builders run `cargo check` and their own tests; the lead runs the combined build, the
  review and the commit.

## A milestone, start to finish

- A milestone is a series of steps (see `ROADMAP.md`).
- Before it closes: run `scripts/measure.sh` on a release build and record the numbers
  in `docs/perf/<milestone>.md` (RSS and CPU idle and under load, latency, binary size,
  leak test). Compare with the previous milestone; investigate any growth.
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
- Unit tested; the whole suite passes.
- Documented where a newcomer would look (view guide, config reference…).
