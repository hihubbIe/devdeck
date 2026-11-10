# macOS manual tests

What the tester tries on the Mac, one section per round of the macOS milestone
(`docs/plans/macos.md`): round 0 now, round 1 after step 3, round 2 after step 6. How to
build and run is in [`docs/BUILDING.md`](../BUILDING.md#macos).

## Round 0

**The question**: does devdeck build on a Mac, does its window open with readable text,
and which tests fail? Most of what touches the OS isn't written for macOS yet, so real
runs, terminals and ports don't work in this round, on purpose (the list below). A
compile error or a crash is a useful result, not a failed round: send it back.

Before you start: `docs/BUILDING.md`, macOS, steps 1 to 6 done (the build finished).
Allow an afternoon; the first builds are long.

### Three starts

Quit between starts with **ctrl+Q** (Control, not ⌘), or by closing the window. Keys are
Linux's in this round: ctrl+/ shows them all, ctrl+K the command palette. A key that
makes macOS do something else instead (Mission Control, switching desktops or input
sources) is worth noting.

**1. No workspace yet**

```bash
target/release/devdeck 2>&1 | tee ~/devdeck-run-1.log
```

A window called **Kugelblitz** shows "No workspace yet", the folder for workspace files
(`~/.config/devdeck/workspaces/`) and an example. Look at the text: sharp, readable, not
cut or overlapping. A small message (a toast) saying the login shell "printed no
environment" may show: note it, it matters later, not now. Take a screenshot.

**2. The demo**

```bash
target/release/devdeck --demo 2>&1 | tee ~/devdeck-run-2.log
```

Sample workspaces with simulated runs: nothing here starts a real process, so almost
everything should work. Try:

- Switch workspace (`work ▾` at the top, or ctrl+alt+1 and ctrl+alt+2).
- The sidebar (ctrl+1 apps, ctrl+3 ports, ctrl+4 layouts, ctrl+6 notes, ctrl+B hides
  it): ↑ and ↓ to move, ⏎ to open.
- Open an app, start a command (⏎), watch its output, stop it (s).
- Drag a tab to another pane or to a pane's edge; scroll a long output with the trackpad.
- Settings (ctrl+,): the size (ctrl+= and ctrl+-) and the theme.
- Write a note and open it again.
- A terminal tab says "The demo has no shell": expected.

Quitting asks first, because the demo's runs are going: confirm. Screenshots of what
looks wrong, and one of the window as a whole.

**3. A real workspace**

Make a workspace file pointing at a folder holding a few of your projects (each child
folder is one project, its commands read from files such as `package.json`, `go.mod`,
`Cargo.toml`, a `Makefile` or a docker compose file). A folder outside Documents and Desktop avoids a macOS permission question;
inside them, macOS may ask whether Terminal may access it: allow it, or devdeck may list
nothing from there.

```bash
mkdir -p ~/.config/devdeck/workspaces
```

Then create `~/.config/devdeck/workspaces/test.toml` with your folder in it:

```toml
name = "test"
folders = ["~/code"]
```

```bash
target/release/devdeck 2>&1 | tee ~/devdeck-run-3.log
```

The projects are listed, alphabetically; open one to see its commands. Starting a command
is refused with "running in a terminal isn't supported on this OS yet": expected. A
toast about the environment may show, as in start 1. Take a screenshot.

### Not yet, on purpose

| What | What you see | Comes with |
|---|---|---|
| Real runs and terminals | refused with "running in a terminal isn't supported on this OS yet" | step 2c |
| The control API (`devdeck-ctl`, `devdeck-mcp`) | "no folder for the control API's socket on this OS yet" | step 2a |
| Leftovers from a previous devdeck | nothing offered | step 2b |
| Ports | nothing listed | step 2d |
| Memory of runs | nothing shown | step 2f |
| ⌘ keys, the Mac title bar, menu bar, Dock | Linux's keys, a plain title bar, no menu of devdeck's | step 4 |

What devdeck writes in this round (layouts, notes) isn't carried over to round 1:
`docs/BUILDING.md`, macOS, step 10.

### Tests expected to fail

From `docs/BUILDING.md`, macOS, step 8. Failures here are expected in this round; send the
whole file anyway, since the exact list is what round 0 is for.

- The app's tests that start real runs or terminals: names containing `pty_driver::`,
  `terminals::`, `ports::`, and the quit tests with runs in `lifecycle::` (step 2c).
- `lifecycle::processes_a_previous_devdeck_left_are_offered_and_killed` and
  `lifecycle::leftovers_left_alone_stay_alone` (step 2b). Each leaves a `sleep 30`
  running for 30 seconds: harmless.
- `api_reads::files_only_inside_the_workspaces_folders` (step 2f).
- `every_editor_key_is_known` (step 4).
- Maybe: the socket tests `stopping_ends_accepting` (in `api::transport`) and
  `shutdown_wakes_a_blocked_accept` (in `devdeck-sys`' `net`); the file watcher's tests
  (`watcher::`).

Anything else that fails is news: that's what to look at first.

### What to send back

In one message or a zipped folder, to the developer:

- **Versions**: the output of `sw_vers`, `uname -m`, `rustc --version`,
  `cargo nextest --version` and `git rev-parse HEAD`; Xcode's version (Xcode menu,
  About Xcode).
- **Files**: `~/devdeck-build.log`; `~/devdeck-run-1.log`, `-2.log`, `-3.log`;
  `~/devdeck-tests.log`; `~/devdeck-debug.log` if devdeck crashed.
- **Screenshots** of the three starts, and of anything that looks wrong.
- **Crash reports**, if devdeck crashed: files named `devdeck…` in
  `~/Library/Logs/DiagnosticReports`.
- **Notes**: what you did, what you expected, what happened, for anything odd; and how
  long the build and the tests took.

Before sending, `git status` in the clone shows no change.
