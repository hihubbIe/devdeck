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

## Keep the step loop fast

- A step's checks run in seconds to a minute: the tests of the crate being changed while
  building, then the whole suite and clippy once before the commit.
- Snapshots: render the states the step adds or touches (the snapshot tool takes a name
  filter), not the whole set.
- Anything long (stress and leak runs of thousands of cycles, long settle times, release
  builds for measurements) runs once at the end of the milestone, never per step.

## A milestone, start to finish

- A milestone is a series of steps (see `ROADMAP.md`).
- Before it closes: run `scripts/measure.sh` on a release build and record the numbers
  in `docs/perf/<milestone>.md` (RSS and CPU idle and under load, latency, binary size,
  leak test). Compare with the previous milestone; investigate any growth.
- At the end of every milestone, work stops and the user tests the app by hand.
  Feedback goes into `REQUIREMENTS.md` / `DESIGN.md` before the next milestone starts.

## Definition of done for a feature

- Works with mouse and with keyboard.
- Every state has been seen on screen: empty, normal, long text (truncates, never
  wraps), error, unavailable with a reason.
- Uses only shared components and theme tokens.
- Unit tested; the whole suite passes.
- Documented where a newcomer would look (view guide, config reference…).
