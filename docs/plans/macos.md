# macOS from source

Asked 2026-11-10 (user): devdeck works on a Mac for someone who builds it from source.
The user has no Mac; a colleague with one builds and tests it, guided by their own coding
agent and the docs written here. Covers R11.6 (the OS layer) and revisits R1.2, R8.2,
R9.6 and R3.8 (`docs/PLATFORMS.md` §3). The inventory, the OS layer's shape and the
recommendations this plan builds on are in `docs/PLATFORMS.md`.

## Decisions (user, 2026-11-10)

- **Apple silicon only**: one target, `aarch64-apple-darwin`. No Intel build.
- **macOS 15 at least.** Apple usually keeps security fixes for the current release and
  the two before it (a habit, not a published policy): 27, 26 and 15 on this date. It
  also settles `LOCAL_PEERTOKEN` (L5), macOS 13 and later per Apple DTS, so the
  `LOCAL_PEERPID` fallback is not needed. GPUI's shaders target 12.0
  (`gpui-pre-apple` `build.rs`: `-mmacosx-version-min=12.0`).
- **Feels like a Mac app**: shortcuts (L10), title bar (L9), menu bar, Dock and last
  window (L13) are part of "works", not extras.
- **`docs/PLATFORMS.md` §5 confirmed**: the crate `devdeck-sys`; config in
  `~/.config/devdeck`, state and notes in `~/Library/Application Support/devdeck`, logs
  in `~/Library/Logs/devdeck`; the socket in `~/Library/Application Support/devdeck/run/`
  when its path fits in 104 bytes, else `/tmp/devdeck-<uid>/`; a transparent title bar
  beside the traffic lights; on macOS devdeck and its runs outlive the last window, ⌘Q
  quits and stops everything.
- **Built on the Mac, not here.** A Mac binary made on Linux needs Apple's macOS SDK and
  a Mac linker (osxcross), whose licence aims at Apple hardware and is read both ways;
  GPUI's shaders need `xcrun metal`; Apple silicon refuses unsigned code; a sent file is
  quarantined by Gatekeeper; and nobody here could run it. Linux only checks that the
  Mac code compiles (below).
- **Order**: this milestone, then packaging the `.app` (L11, §5: `macos-15` runner,
  ad-hoc signing), then CI.

## What is checked where

- **Here, on Linux**: the whole suite and clippy, as always; the Linux side of every
  step. For Mac code, only the crates without GPUI and without C code ("The Mac compile
  check" below): `cargo check --target aarch64-apple-darwin -p devdeck-sys -p
  devdeck-core -p devdeck-api -p devdeck-ctl -p devdeck-mcp` (the target is installed,
  2026-11-10; `-p` is right here, `docs/PLATFORMS.md` §4). Compile errors only, nothing
  about linking or running.
- **Mac code in `devdeck-app` and `devdeck-ui` can't be compiled here**, so it is written
  to be compiled on Linux too: GPUI's own cross-platform calls (`set_menus`,
  `TitlebarOptions`, `on_reopen`) chosen with `cfg!(target_os = "macos")` in an
  expression, which type-checks both branches on every OS, rather than `#[cfg(…)]` on
  items, which Linux never compiles. Where an item has to be Mac-only, it is small and
  listed in the step's "as built" for round 1 to watch.
- **On the Mac, by the tester**: the build itself, the whole suite (each capability's
  conformance tests run the macOS backend for real), the "to verify on a Mac" items as
  tests, and the manual test.

## Steps (one commit each)

Each step's own plan is added to this file ("Step N plan") before it is built, then
reviewed and assessed (`docs/WORKFLOW.md`); "Step N, as built" after.

0. **Inventory and requirements.** Refresh `docs/PLATFORMS.md` §1–2 against the code
   (commit `9f40aa0` then; 19 commits since, line numbers have moved). Known gaps:
   `api/handlers.rs` (`file.read` checks the opened file through `/proc/self/fd/<fd>`:
   Linux only, every read refused on macOS; `F_GETPATH` with `fcntl` is the likely
   equivalent, to verify), `api/handlers.rs` reading `XDG_RUNTIME_DIR` (L6),
   `api/transport.rs` (`libc::recv` with `MSG_PEEK | MSG_DONTWAIT`, Linux and macOS, from
   phase 3 step 5 of the control API). Requirements: R1.2 gains macOS 15+ on Apple
   silicon; R8.2 says stopping is exact on Linux, best effort on macOS (L2).
1. **`devdeck-sys`, Linux only.** The OS code moves into the crate with no change of
   behaviour; the whole suite proves it. One commit per part:
   - 1a. The crate, `Capability`, `Support`, `support()`, `SysError`; the small
     capabilities: `dirs/` (L6, L7), `memory/` (L8), `time/` (U8), `fs/` (U5, U6, and the
     open file's path from step 0), `signals/` (U4).
     Also a logger, so GPUI's `log` warnings reach the tester (review point 13): our own
     `log::Log` in `crates/devdeck/src/main.rs`, installed first thing, not in tests; one
     line per record on stderr (level, target, message); warnings and errors by default,
     `DEVDECK_LOG=info|debug|trace` for more. `log` 0.4.35 is already in the tree through
     GPUI, so no new crate (R11.3); `env_logger` would bring crates of its own for
     per-module filters nobody needs here. Not covered: kube's `tracing` events (unless
     `tracing`'s `log` feature is on, unchecked; the cluster isn't in the Mac rounds), and
     a log file in `~/Library/Logs/devdeck` (with packaging: an app started from Finder
     has no stderr).
   - 1b. `procs/` (L3) and `ports/` (L4).
   - 1c. `peer/` (L5).
   - 1d. `child/` (L1, L2, U2, U3): most of `devdeck-core/src/process.rs` and `scope.rs`.

   Each with its `unsupported.rs`, its conformance tests in `mod.rs`, and a seam the
   app's tests replace with a fake (L12). The guards from `docs/PLATFORMS.md` §4:
   `rustix`, `libc`, `nix` and `signal-hook` leave every other crate's `Cargo.toml`; a
   source scan test fails on `/proc`, `/sys/`, `XDG_RUNTIME_DIR`, `systemd-run` or
   `std::os::linux` outside `devdeck-sys`. The clients (`devdeck-api`, `devdeck-ctl`,
   `devdeck-mcp`) take the socket's folder from `devdeck-sys` (L6). How tests that need
   an OS call (a fifo in `tests/app/api_reads.rs`) get it is decided in 1a's plan.
2. **The macOS backends**, one capability per commit, each passing its conformance
   tests on the Mac and the Mac compile check here:
   - 2a. `dirs/` (L6, L7): the folders above, the 104-byte socket path.
   - 2b. `child/` (U2, L1, L2): `FD_CLOEXEC` set with `fcntl` after `openpt`; one
     `kqueue` with the terminal, a wake pipe and `EVFILT_PROC`/`NOTE_EXIT`, then
     `waitpid`; stopping the tree is best effort (own session, `killpg`, then the
     descendants found through `procs/`), shown as `Partial` where it matters.
   - 2c. `procs/` (L3): `proc_listallpids`, `proc_pidinfo(PROC_PIDTBSDINFO)`,
     `proc_name`, `sysctl(KERN_PROCARGS2)`.
   - 2d. `ports/` (L4): `proc_pidinfo(PROC_PIDLISTFDS)`, then
     `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` per descriptor.
   - 2e. `peer/` (L5): `nix` (macOS only): `getpeereid`, `LOCAL_PEERTOKEN`.
   - 2f. `memory/` (L8): `task_info(TASK_VM_INFO)`, `phys_footprint`; `fs/`'s open
     file path.

   Anything that cannot work is `No(why)` and shown disabled with its reason (R5.1).
3. **The build guide and round 1's test guide** (below). Then everything is pushed, with
   the user's OK, and the tester runs round 1.
4. **Mac behaviour in the app.** L10: app keys through gpui's `secondary-` modifier, a
   macOS keymap where a key differs, `ctrl-…` left to the terminal's program, ⌘C/⌘V in
   the terminal; where gpui-component switches its Input bindings per OS (to verify,
   `devdeck-ui/src/code.rs` `EDITOR_KEYS`). L9: `TitlebarOptions` per OS, our content
   beside the traffic lights as Zed does it. L13: the menu bar (devdeck: About,
   Settings, Hide ⌘H, Quit ⌘Q; Window), a Dock click reopens the window as it was, the
   last window's close per the decision above. Also round 1's findings.
5. **What Linux shaped** (`docs/PLATFORMS.md` §3): path comparisons in discovery, files
   and notes on case-insensitive APFS; `/usr/bin/git` without the Command Line Tools
   (the git panel says git is missing); R9.6 against a trackpad (by the tester); window
   positions and R3.8 (dragging tabs between windows) stay as they are unless round 2
   says otherwise.
6. **Round 2's test guide**, pushed with the user's OK; the tester runs it. The
   milestone ends there (R11.5).

## Rounds

The first Mac build will find what Linux could not, so the tester runs two rounds:

- **Round 1**, after step 3: does it build, do the tests pass, does it start. Cheap to
  fix early: steps 4–5 build on the backends.
- **Round 2**, after step 6: the full manual test.

Between and after rounds, the tester's report is fixed here; each fix gets a regression
test where the suite can hold one.

## The build guide (step 3)

`docs/BUILDING.md` gets a **macOS** section, whole on its own so the tester's agent reads
one place; today's Linux setup becomes a **Linux** section; the shared parts (tests and
lints, the build folder) stay. `README.md` gets a line pointing Mac users to it, and the
project `CLAUDE.md` names it among the docs to read.

Written for someone who doesn't know Rust, read with their coding agent: one line at the
top says so. Every step has the same four parts: what it is for (a line); the command, in
a `bash` block, with the official page it comes from; how to check it worked (a command
and what it prints); if it fails, the exact error text and its fix. No command from
memory. Rules for the agent: cargo from the repo root with `--workspace`, never `-p`;
never `scripts/install.sh` (Linux only); `sudo` only where the guide says.

Steps of the guide:

1. **The Mac.** `uname -m` prints `arm64`; `sw_vers` shows 15 or later. Disk and memory
   from "The build folder" (tests 6.2 GB, peak memory 5.2 GB cold, measured on Linux,
   said so).
2. **Xcode**, the full app, not only the Command Line Tools (GPUI compiles its shaders
   with `xcrun metal` at build time; Zed's guide asks for Xcode). From the App Store,
   launched once with the default components. Then `xcode-select --install`,
   `sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer`,
   `sudo xcodebuild -license accept`. On macOS 26 and later, if the shaders don't
   compile: `xcodebuild -downloadComponent MetalToolchain`, and if that fails
   `xcodebuild -runFirstLaunch` then again (Zed's guide).
3. **Rust**: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
   (rust-lang.org); a new terminal; `rustc --version` shows 1.99 or later.
4. **nextest**: `curl -LsSf https://get.nexte.st/latest/mac | tar zxf - -C
   ${CARGO_HOME:-~/.cargo}/bin` (nexte.st, a universal binary); `cargo nextest --version`.
5. **The code**: `git clone` from GitHub (git comes with Xcode), into its folder.
6. **Build**: `cargo build --release` (it also builds `devdeck-ctl` and `devdeck-mcp`);
   the first build is long.
7. **Run**: `target/release/devdeck`. What to expect: the window and the menu bar; no
   app icon of our own in the Dock until there is a `.app`; possibly macOS asking for
   access to Documents or Desktop, credited to Terminal since it started devdeck (to
   verify).
8. **Tests**: `cargo nextest run --workspace --no-fail-fast`, output saved to a file. To
   send back: that file, `sw_vers`, `git rev-parse HEAD`.
9. **The manual test**: a link to the round's test guide.
10. **Update**: `git pull`, build again; `cargo clean` after a `rustup update`, and why.
11. **devdeck's files on a Mac**: where config, state, notes and logs are; how to reset
    them; how to remove everything.

Troubleshooting, from Zed's guide and the pages above, growing with round 1:

| Error | Fix |
|---|---|
| `xcrun: error: unable to find utility "metal"` | step 2's `xcode-select --switch`, then the Metal toolchain |
| `command not found: cargo` | a new terminal |
| "dependency is using unstable features" | `cargo clean`, build again |
| `Too many open files (os error 24)` | nextest (step 8 uses it) |
| licence not accepted | `sudo xcodebuild -license accept` |

The guide says it has not yet been run on a Mac until round 1 confirms it.

Sources: [Zed, Building Zed for macOS](https://zed.dev/docs/development/macos),
[rust-lang.org, Install Rust](https://rust-lang.org/tools/install),
[nexte.st, pre-built binaries](https://nexte.st/docs/installation/pre-built-binaries/)
(read 2026-11-10).

## The test guides (steps 3 and 6)

`docs/plans/macos-manual-test.md`, as `M1-manual-test.md` and `M2-manual-test.md`: what
to try, what should happen, what to send back (the test output, devdeck's log,
screenshots).

- **Round 1**: the build, the whole suite, devdeck starts and shows the apps of a
  workspace folder, a run starts, shows its output, stops, and leaves no process behind
  (`ps`); its ports show; `devdeck-ctl` reaches the control API; a note saves; a file
  opens.
- **Round 2**: round 1 again, then ⌘ shortcuts and the terminal's ctrl keys, copy and
  paste, the title bar, the menu bar, Hide and Quit, closing the last window and the Dock
  click, trackpad scrolling (R9.6), two paths that differ only in case, git without the
  Command Line Tools if the tester can spare a user account for it.

## Out of scope

The `.app` bundle, its icon, signing and notarization (L11, after this milestone); CI;
Intel Macs; Windows; window positions and dragging tabs between windows (step 5 only
records round 2's view); measuring devdeck on the Mac (`scripts/measure.sh` is Linux's;
R11.4 is measured on Linux as for every milestone, and step 1 must not cost anything
there).

## States to check visually

On Linux, after each step: nothing changes. On the Mac, by the tester (round 2): an
action unsupported on macOS shown disabled with its reason (palette and menus); stop's
best-effort note (L2); the title bar with a long workspace name (truncates, R4.4); the
menu bar; the window after a Dock click; the git panel without git.

## Tests

- Conformance tests per capability, in `devdeck-sys` (`docs/PLATFORMS.md` §4): spawn `sh`
  in a terminal, read its output, find its tree, find a port it opens, stop it, identify
  a peer on a socket pair, the folders, the socket path limit, the open file's path.
- The "to verify on a Mac" items as tests where they can be: `poll` on a terminal fd (a
  test that records the answer, not one devdeck relies on), `LOCAL_PEERTOKEN`, the open
  file's path through `fcntl`.
- The source scan test and the dependency guard (step 1).
- The app's tests on fakes instead of `/proc` (L12), the same on both OSes.

## Unsure

- Whether GPUI's `secondary-`, `set_menus`, `on_reopen` and `TitlebarOptions` compile
  and behave the same on Linux as on macOS for the `cfg!` rule above (read in GPUI's
  source at step 4's plan).
- `F_GETPATH` for the open file's path (step 0).
- Whether macOS asks for access to Documents or Desktop when discovery reads them, and
  who it names (guide step 7).
- Whether GPUI's test platform runs the app's tests on a Mac as on Linux.
- How Zed places its title bar content beside the traffic lights (L9).
- `libc`'s coverage of the `proc_pidfdinfo` socket structs (L4).

## The Mac compile check (2026-11-10)

`cargo check --workspace --target aarch64-apple-darwin --keep-going`, at `ed19d2a`, on
Linux with the target installed and nothing else:

- **`devdeck-api`, `devdeck-ctl`, `devdeck-mcp`: compile.**
- **`devdeck-core`: 9 errors, all known "No build" items**: `eventfd`, `EventfdFlags`,
  `pidfd_open` and `WaitId::PidFd` in `process.rs` (L1), `pidfd_open`, `PidfdFlags` and
  `pidfd_send_signal` in `scope.rs` (L2), `OpenptFlags::CLOEXEC` in `process.rs` (U2).
  Nothing the inventory missed.
- **C code fails to build**: `ring` (through `devdeck-kube`), `psm`, `tree-sitter` and
  every grammar. Their build scripts call the host's `cc` (GCC), which refuses Apple's
  flags (`-arch`, `-mmacosx-version-min`). A Mac C compiler (clang) would get further;
  the C headers would then come from Apple's SDK, which is not used here (Decisions).
- **`gpui-pre-apple` fails**: its build script runs only when built on a Mac, so the
  `shaders.metallib` that `metal_renderer.rs` includes is never made on Linux. So
  `devdeck-ui`, `devdeck-app`, `devdeck-kube` and the binary are never reached.

Hence the check above, by crate, and the `cfg!` rule for the app's Mac code.

## Plan review (subagent, 2026-11-10), assessed

Where a point is taken, it overrides the sections above. Points 1, 3, 6 (the test sweep)
and 9 were checked again in the sources before being taken.

1. **No text on macOS**: `gpui-pre-macos` uses `AppleTextSystem` only with its
   `font-kit` feature, else `NoopTextSystem` ("no text will be rendered",
   `platform.rs:220-231`); our `gpui_platform` features are `wayland` and `x11` only.
   Taken: `font-kit` joins them in step 1a.
2. **L13 under-designed; quit from outside the menu skips R8.2**: the Dock's Quit, logout
   and shutdown reach only `on_app_quit` observers, and ours only closes the API; the
   quit guard, the leftovers dialog, the API's present calls, approvals and detached
   windows all assume a main window. Taken: an `on_app_quit` that stops everything as the
   signals do (step 1a, it holds on Linux too); step 4 gets a design part of its own for
   devdeck with no main window (reopen restores the window, never a second one;
   `on_reopen` is set on `Application` in `main.rs`; ⌘Q with runs and no window; where an
   approval shows). Tests never reach `TestPlatform::on_reopen` or `hide`
   (`unimplemented!`).
3. **`secondary-` breaks key helpers silently, on Linux too**: `Keystroke::parse` refuses
   `secondary-` without an OS, so `terminal_takes`, `editor_takes`, `GIVEN_BACK` and the
   stress keys would drop their guards, and `key_text` would show "secondary". Taken:
   every parse goes through `Keystroke::parse_with_os` with `cx.platform_info().os`; the
   app's Mac branches (keys, menus, title bar) are chosen at run time with
   `cx.platform_info()`, not `cfg!`, so Linux tests run them through
   `TestAppContext::set_platform_info`. This replaces the `cfg!` rule above and settles
   its "Unsure" item.
4. **Option key**: every `alt` key goes to the app (`core::key_bytes`), and non-US layouts
   type `@ [ ] { } | ~ \` with Option. Taken: step 4 decides Option's characters and
   "Option as Meta"; round 2 tries a non-US layout. The GPUI detail is to verify.
5. **2b uses `procs/` before 2c builds it**. Taken: the order becomes 2a `dirs/`, 2b
   `procs/`, 2c `child/`, 2d `ports/`, 2e `peer/`, 2f `memory/` and `fs/`.
6. **Round 1 failures not planned**. Taken:
   - `testing.rs`'s sweep reads `/proc/<pid>` and would delete the folders of tests
     running beside it: it moves to `procs::alive` in step 1b.
   - `EDITOR_KEYS` must follow gpui-component's per-OS Input bindings (`#[cfg]` in
     `gpui-base`'s `input/base/state.rs`, which answers that "to verify" item): per OS in
     step 4; `every_editor_key_is_known` is listed in round 1's guide as an expected
     failure until then.
   - `text_never_wraps_at_large_scale` starts the real platform off the main thread:
     step 0 checks how, and it runs on Linux only if it can't run on a Mac.
   - `$TMPDIR` under `/var/folders`, `/tmp` and `/var` as links to `/private`, and socket
     paths near 104 bytes in tests: step 5's path audit, and known risks in round 1's
     guide.
7. **Requirements and rules drift**. Taken: step 0 also updates R10.1 and R13.3, and
   `docs/PLATFORMS.md` §4's "CI comes before the first macOS backend" (CI now comes after
   packaging, by the user's choice). Step plans stay in this file, as the control API's
   phases did. One test guide, `docs/plans/macos-manual-test.md`, with a section per
   round.
8. **Inventory gaps**: `process.rs:249` (`/proc/{group}/comm`), `api/mod.rs:227`,
   `api/transport.rs:61` and `:201` (no module yet: a `net/` capability), U3 in `git.rs`
   and `environment.rs`, `Errno::LOOP`, `libc::mkfifo` in a test, the `XDG_RUNTIME_DIR`
   error texts in `devdeck-ctl` and `devdeck-mcp`, and `env -0` (`environment.rs:30`),
   which older macOS `env` lacked. Taken: all into step 0's inventory; `env -0` gets a
   conformance test and a "to verify" line.
9. **`libc` 0.2.191 lacks** `socket_fdinfo`, `PROC_PIDFDSOCKETINFO`, `task_vm_info` and
   `TASK_VM_INFO`; it has `proc_pid_rusage` with `ri_phys_footprint`. Taken: L8 uses
   `proc_pid_rusage`; 2d's plan chooses between our own `repr(C)` structs (with a size
   test on the Mac) and the `listeners` crate.
10. **The Linux check skips test code and lints**. Taken: `cargo clippy --target
    aarch64-apple-darwin -p devdeck-sys -p devdeck-core -p devdeck-api -p devdeck-ctl -p
    devdeck-mcp --all-targets -- -D warnings` replaces the `cargo check` above.
11. **The first Mac build could come right after step 1**. Taken: during step 1 macOS
    takes `unsupported.rs`, so the whole app builds on a Mac; a short **round 0** after
    step 1 (Xcode, the build, the window with its text, the build guide's first draft)
    catches setup and GPUI problems before the six backends. Runs don't start in round 0;
    the guide says so. The build guide's draft moves to step 1, finished in step 3.
12. **R13.11**: the socket's folder is resolved in `devdeck-api` (`default_socket`), which
    calls `devdeck-sys`; `devdeck-ctl` and `devdeck-mcp` don't. Taken.
13. **"devdeck's log" doesn't exist**: devdeck installs no logger, so GPUI's `log`
    warnings are lost. Taken for the guides: the tester sends the build log,
    `RUST_BACKTRACE=1 target/release/devdeck 2>&1 | tee …`, crash reports from
    `~/Library/Logs/DiagnosticReports`, `xcodebuild -version`, `rustc -vV`, `cargo
    nextest --version`; closing the Terminal window that started devdeck quits it
    (SIGHUP); round 1 brings a sample workspace config and a small project with a
    server. A logger: decided by the user, 2026-11-10, our own (step 1a).
14. **State and notes share a folder on macOS** (both use `workspaces/<id>/`). Taken:
    `…/devdeck/state` and `…/devdeck/notes`, still under Application Support as the user
    confirmed; the guide's reset never deletes notes.
15. **Native menus can't show a reason**. Taken: menu bar items come from the registry,
    greyed out through GPUI's menu validation; the reason shows in the palette and
    devdeck's own menus.
16. **States missing**. Taken, for round 2 and step 4's own plan: no window with runs
    going, ⌘Q and the Dock's Quit then; a Dock click after minimize, ⌘H and close; full
    screen; a detached window's buttons beside the traffic lights; an approval or present
    call with no window; the leftovers dialog after `kill -9` of devdeck; the quit guard
    on ⌘Q; key hints showing ⌘; Option characters on a non-US layout.
17. **Build guide details**. Taken: disk for Xcode and the Metal toolchain; `xcode-select
    --install` saying the tools are already there is fine; the repository is public, so
    cloning needs no account.

## Step 0, as built (2026-11-10)

Docs only; no code, no OS touched. Its plan was the step's entry above with review
points 6–8 and 10; the review below is of the result, against the code.

- `docs/PLATFORMS.md` §1–2 checked line by line against `af9fad4` (a search for `/proc`,
  `/sys`, `XDG_`, `rustix::`, `libc::`, `signal_hook`, `std::os::`, `cfg(unix)`,
  `systemd`, `/bin/sh`, `"SHELL"`, `"HOME"`, `Errno::`, `env -0` over every crate, tests
  included; it missed `${SHELL` and `scripts/`, which the review below found). New rows: **L14** (an open file's path, `file.read`) and **U11** (socket
  shutdown and peek); new places in L3, L4, L6, L10, L12, L13, U1, U6, U7, U10; what
  `libc` 0.2.191 has and lacks in L3, L4, L8, L14; the `font-kit` feature in "Not
  affected".
- §3: `$TMPDIR` and `/private` paths. §4: `net/`, `fs/` takes L14; the Mac check on
  Linux by hand, with clippy and `--all-targets` (review point 10); CI after the
  milestone and packaging; why the whole workspace can't be checked from Linux. §5:
  state and notes in subfolders (review point 14), logs with packaging. §6: the Zed
  `env -0` commit.
- `docs/REQUIREMENTS.md`: R1.2 gains macOS 15 on Apple silicon; R8.2 exact on Linux,
  best effort on macOS (now `[~]`); R10.1 the macOS folders; R13.3 the macOS socket
  folder (now `[~]`).
- `text_never_wraps_at_large_scale` (review point 6): it calls
  `gpui_platform::current_platform(true)`, which on macOS is `MacPlatform::new`, which
  panics off the main thread ("Mac platform not created on main thread"). libtest likely
  runs a test on a thread of its own (rust-src isn't installed here to read it). Step 1a
  makes the test Linux-only with that reason, unless round 0 shows it passes.

### Step 0 review (subagent), assessed

Each reference below was checked again in the code before being taken. All taken; the
fixes are in the step's commit.

1. R8.1 and R8.2 didn't say what L13 says (the Dock's Quit, logout and shutdown). Taken:
   both name it; §3 lists R8.1.
2. U7 missed `SHELL_LINE` (`runner.rs:55`, every shell tab): the search's `$SHELL`
   didn't match `${SHELL`. Taken, and the search list above corrected.
3. L12 missed `tests/app/ports.rs` (the real port scan) and `process.rs:862` (the
   `setsid` program). Taken.
4. `scripts/` half covered: `measure.py` (R11.4: `/proc`, `/sys/fs/cgroup`, `xdotool`),
   `drag_experiment.py`, and `~/.local/bin` in `devdeck-mcp/src/main.rs:2`. Taken, in L11.
5. L9's references were off and the frame code dropped. Taken: `:147-185`, `:737-861`,
   `:1039-1170`.
6. L10 said every parse goes through `parse_with_os`; none does. Taken: a to-do for step
   4; `GIVEN_BACK` is defined at `code.rs:37`.
7. `sun_path`'s 104 bytes include the NUL, and the limit is on the socket's path, not the
   folder's. Taken in L6, §5 and R13.3: the socket's path fits in 103 bytes.
8. R5.3, R5.4, R6.12, R7.3 and R12.3 name ctrl keys. Taken: one line in R5.1, one in §3.
9. R3.8, R9.8 and R1.1 become ambiguous on macOS. Taken: listed in §3 (R1.1's `.app`
   comes with packaging, after this milestone).
10. L1 didn't cite the wake's write and read (an eventfd's counter; a pipe must be drained
    whole). Taken.
11. Whether `XDG_*_HOME` count on macOS; L7's ranges stopped a line short. Taken: ranges
    widened; §5 says `DEVDECK_*_DIR` first everywhere and leaves `XDG_*` on macOS to step
    2a's plan.
12. L5: 77 is right on mips (only sparc differs); `libc` already has `getpeereid`,
    `LOCAL_PEERPID` and `LOCAL_PEERTOKEN`, only `audit_token_t` is missing. Taken in L5;
    nix or libc is chosen in step 2e's plan.
13. U1, U3 and U6 missed sites (tests, `git.rs:796`, the calls at `git.rs:830,836`).
    Taken.
14. The macOS socket path has a space, and shells get it as `DEVDECK_SOCKET`. Taken: a
    line in §3.

## Step 1a plan (2026-11-10): `devdeck-sys`, the small capabilities

The crate, and six capabilities moved into it with no change on Linux; with them, the
three things round 0 needs on a Mac (text, quitting from outside devdeck, a log). R11.6,
R8.2, R13.3; L6, L7, L8, L14, U4, U5, U6, U8, U11 (`docs/PLATFORMS.md`).

On macOS during step 1: what is portable Unix code already (folders, local time, locks,
signals, sockets) goes in a `unix.rs` both OSes use; memory and an open file's path are
unsupported until step 2f; the API's socket folder until step 2a. So in round 0 the API
is off on a Mac with its reason, and `file.read` is refused.

### The crate

`crates/devdeck-sys`, `doctest = false`. `lib.rs`: `SysError` (`Unsupported(&'static
str)`, `Io(io::Error)`), `Support` (`Yes`, `Partial(why)`, `No(why)`), `Capability`,
`support(Capability)`, computed once and cached. Each module: `mod.rs` (the functions
devdeck calls, documented once, and the conformance tests), and as needed `linux.rs`,
`unix.rs`, `unsupported.rs`, picked by `cfg_attr` on the target (`docs/PLATFORMS.md` §4).

| Module | devdeck calls | Moves from | Linux | macOS in step 1 |
|---|---|---|---|---|
| `dirs/` | `config(env)`, `state(env)`, `data(env)`; `runtime()`, the socket folder with `devdeck` in it; `SOCKET_PATH_MAX` | `devdeck-core/src/settings.rs:63`, `state.rs:15`, `notes.rs:18`; `devdeck-app/src/api/mod.rs:89-90,234-235`, `devdeck/src/main.rs:82`, `devdeck-api/src/client.rs:14-19`, `api/handlers.rs:766`, `api/socket.rs:13,51` | as today | `unix.rs` for the three folders; `runtime()` unsupported |
| `memory/` | `footprint()` | `devdeck-app/src/stats.rs:19-28`, its parser and tests | as today | unsupported (2f) |
| `time/` | `local_hms(ms)` | `devdeck-app/src/settings_view.rs:44-47` | `unix.rs` | `unix.rs` |
| `fs/` | `lock(&File)`, `open_no_follow(path, nonblocking)`, `is_mine(&Metadata)`, `is_loop(&io::Error)`, `open_path(&File)` (L14), `make_fifo(path)` | `api/socket.rs:6,54,76`, `lifecycle.rs:39`, `api/handlers.rs:818-839`, `devdeck-core/src/toolchains/project.rs:116`, `tests/app/api_reads.rs:141` | as today | `unix.rs`; `open_path` unsupported (2f) |
| `signals/` | `quit_signals()`: SIGTERM, SIGHUP, SIGINT, as an iterator | `devdeck-app/src/lifecycle.rs:81-82` | `unix.rs` | `unix.rs` |
| `net/` | `shutdown(&OwnedFd)`, `hung_up(&UnixStream)` | `devdeck-app/src/api/transport.rs:61,201` | `unix.rs` | `unix.rs` |

- `devdeck-core` keeps its public folder functions, each one line into `devdeck-sys`;
  their callers don't change.
- The socket folder comes whole from `dirs::runtime()`: `socket::bind` stops adding
  `devdeck`; `api::Config.runtime` becomes `Result<PathBuf, String>`, the reason from
  `devdeck-sys`. On Linux it still names `XDG_RUNTIME_DIR` (`tests/app/api.rs:148`
  passes there).
- `devdeck-api`'s `default_socket` calls `dirs::runtime()`: `devdeck-api` depends on
  `devdeck-sys`; `devdeck-ctl` and `devdeck-mcp` still depend on `devdeck-api` only
  (R13.11). Their error texts give the reason instead of naming `XDG_RUNTIME_DIR`.
- `make_fifo` is a plain public function, so `devdeck-app` doesn't keep `libc` for a
  test.

### What round 0 needs

1. **Text on a Mac**: `font-kit` joins `gpui_platform`'s features in the root
   `Cargo.toml`. Nothing changes on Linux: it only enables `gpui_macos`'s feature, not
   built there. `Cargo.lock` gains font-kit's crates for macOS.
2. **Quitting from outside devdeck** (the Dock's Quit, logout, shutdown): an
   `on_app_quit` observer installed with the shell calls `kill_to_quit`. It does nothing
   when the quit went through the shell (`quits > 0`: no second kill marking runs to
   start again after "Stop all and quit") and in tests (`quit_for_real` off: every app
   test ends with GPUI's quit). GPUI calls the observers from its quit path on every
   platform (`gpui-pre` `app.rs:1212`), so it runs on Linux too.
3. **The logger**: a `logger` module in `devdeck-app`, installed first thing by
   `main.rs`; stderr, one line per record (level, target, message); warnings and errors
   by default, `DEVDECK_LOG=info|debug|trace` for more. `log` 0.4.35, already in the
   tree. In the library so its level parsing and line format are tested.

### Also

- `text_never_wraps_at_large_scale` is `#[cfg(target_os = "linux")]`, with the reason:
  on a Mac GPUI's real platform must start on the main thread.
- `devdeck-core` drops `signal-hook` (in its `Cargo.toml`, never used: only a comment
  names it). `devdeck-app` drops `signal-hook`; it keeps `rustix` and `libc` until 1c.
- **Source scan test**, `crates/devdeck-sys/tests/sys/main.rs`: outside `devdeck-sys`,
  code lines (comments skipped) never use `/proc`, `/sys/`, `XDG_RUNTIME_DIR`,
  `systemd-run`, `std::os::linux`, `rustix::`, `libc::` or `signal_hook`. Files that
  1b–1d move are allowed, each with the step that empties it (`process.rs`, `scope.rs`,
  `procfs.rs`, `ports.rs`, `identity.rs`, `port_scan.rs`, `lifecycle.rs`, `git.rs`,
  `environment.rs`, the tests of L12); empty after 1d. It also fails when a listed file
  needs its place no more, so the list can't go stale.
- **The Mac check** on Linux: `cargo clippy --target aarch64-apple-darwin -p
  devdeck-sys -p devdeck-api -p devdeck-ctl -p devdeck-mcp --all-targets -- -D
  warnings`. `devdeck-core` joins after 1d, when its process code has moved; that is
  also when the whole app first builds on a Mac (round 0).
- **Docs**: `docs/DESIGN.md:82` (the crate list) gains `devdeck-sys`; `:382-386` (the
  clients depend only on `devdeck-api`) adds "which uses `devdeck-sys` for the socket's
  folder"; `docs/PLATFORMS.md` §4 marks the six modules done.

### Tests

- Conformance, in each `mod.rs`: `dirs`, the precedence tests moved from core,
  `runtime()` with `XDG_RUNTIME_DIR` set and unset, the socket path limit; `memory`, a
  footprint above zero on Linux, unsupported elsewhere; `time`, a known instant with
  `TZ` set (nextest runs each test in a process of its own); `fs`, a second lock fails,
  `open_path` gives the file's real path, a link loop is a loop, a FIFO opens without
  blocking; `signals`, a SIGHUP sent to ourselves comes through; `net`, on a socket pair
  `hung_up` is false, then true once the other end is dropped.
- The quit observer (`tests/app/lifecycle.rs`): with `quit_for_real` and nothing quit, a
  run's process is gone after `cx.quit()`; with `quits > 0`, no second kill.
- The logger: level parsing (`DEVDECK_LOG` empty, unknown, each level), the line format.
- The source scan test; the whole suite and clippy on Linux; the Mac check.

### States to check visually

On Linux nothing changes on screen. With `XDG_RUNTIME_DIR` unset, Settings shows the API
off with its reason, short and truncated (R4.4).

### Out of scope

The process table, ports, the API's callers, running things (1b–1d); any macOS backend
(step 2); removing `rustix` and `libc` from `devdeck-core` and `devdeck-app` (1d); the
menu bar, keys and title bar (step 4).

### Unsure

- Which memory figures a Mac shows (it has one, the physical footprint): 2f.
- Whether `XDG_*_HOME` count on a Mac: 2a; step 1's `unix.rs` keeps today's behaviour.

### Step 1a plan review (subagent), assessed

Where a point is taken, it overrides the plan above. Points 2, 3 and 10 were checked
again in the sources (GPUI's test macro, `TestPlatform::quit`, `Cargo.lock`, every
`api::Config` built).

1. `dirs::runtime()` took no `env`, unlike the three folders, so `default_socket`'s fake
   env tests and the planned test would set the real environment. Taken:
   `runtime(env)`, and `default_runtime()` for the real one, as the folders do.
2. The socket folder change breaks tests the plan didn't list. Taken: `socket::bind`
   takes the socket folder itself; its unit tests (`api/socket.rs:100-126`) and
   `tests/app/api.rs:88,93` make that folder 0700 themselves; every `Config { runtime:
   None }` (`tests/app/api.rs:118,132,146`, `api_runs.rs:142`, `api_events.rs:20`,
   `settings.rs:133`, `bin/snapshot.rs:990,1001`) gives `Err` with the reason from
   `devdeck_sys::dirs::runtime(|_| None)`, so `api.rs:148` checks `devdeck-sys`'s
   reason, not its own. `default_socket` returns a `Result`; `devdeck-mcp`'s
   `Bridge::new` (`lib.rs:141,146`, its test `:284`) and `devdeck-ctl/src/main.rs:171-172`
   follow.
3. The quit observer's `quits > 0` guard misses the signal path (`lifecycle.rs:101-104`)
   and the window-closed fallback (`main.rs:44-52`): both kill and then `cx.quit()`, and
   the observer would kill again; and `#[gpui::test]`'s teardown calls `App::quit`, a
   no-op on the test platform (`gpui-pre-macros` `test.rs:172`, `TestPlatform::quit`),
   so observers don't run there. Taken: a `killed` flag on the shell, set by
   `kill_to_quit`, `kill_everything` and `quit_now`; the observer runs only when it isn't
   set and `quit_for_real` is on (the snapshot tool's `HeadlessAppContext` runs
   observers when dropped, with `quit_for_real` off). Its test calls
   `TestAppContext::quit()` directly; "no second kill" is seen through a count of kills.
4. The scan's allow-list would fail at once: `devdeck-core/src/testing.rs:63` isn't
   listed; `port_scan.rs` and `tests/app/ports.rs` match no pattern; the files scanned
   aren't said. Taken: it scans `crates/*/{src,tests,benches}/**/*.rs` and `build.rs`,
   not `crates/devdeck-sys`, `target/` or `scripts/`; a line is skipped when it starts
   with `//` once trimmed; the list has `testing.rs` (1b) and no file that matches
   nothing; `client.rs`'s `XDG_RUNTIME_DIR` tests move to `devdeck-sys` with
   `runtime(env)`. It is a `#[cfg(test)]` module of `devdeck-sys`'s library: one test
   binary less to link.
5. `devdeck-sys`'s tests have no temp folder, and a dev-dependency on core would be a
   cycle and break the Mac check. Taken: a small `#[cfg(test)]` `TempDir` in
   `devdeck-sys`, under the same `/tmp/devdeck-tests/<pid>/` (core's sweep still clears
   crashed runs).
6. `shutdown` on a listening socket may not wake a blocked `accept` on macOS (from
   memory of XNU). Taken as a conformance test: shutdown wakes a blocked `accept` within
   1 s. If it fails on the Mac (round 0 runs `devdeck-sys`'s tests), `net/macos.rs`
   connects once to the socket after setting the flag.
7. `memory` must keep rss, pss and uss on Linux: `main.rs:155-166` and `stress.rs:54,281`
   print them and `scripts/measure.py:434` parses them (R11.4). Taken: `Memory` stays as
   it is; the smaps parser lives in `linux.rs` (else it is dead code on macOS and fails
   the Mac clippy); macOS's fields are 2f's.
8. `open_no_follow(path, nonblocking)` doesn't fit `socket.rs:54` (create, write, mode
   0600). Taken: `no_follow(&mut OpenOptions, nonblocking)` adds the flags to the
   caller's options.
9. The logger must call `log::set_max_level` (the default is off); `log` becomes a
   workspace dependency of `devdeck-app`; on Linux stderr gains GPUI's, wgpu's and
   calloop's warnings, silent until now. Taken: that is a change on Linux, wanted (the
   same warnings are what round 0 needs); one start is checked by eye before the
   commit. `DEVDECK_LOG` is listed in `main.rs`'s header and in `docs/BUILDING.md`.
10. `font-kit`'s crates are in `Cargo.lock` already (`zed-font-kit`, `core-text`, through
    `gpui-pre-apple` and others). Taken: no lock change; the cost is one rebuild of
    `gpui-pre-platform` and what is above it.
11. "Removing `rustix` and `libc` from `devdeck-app` (1d)" contradicts "until 1c": after
    1a only `api/identity.rs` uses them. Taken: 1c.
12. `devdecks_own` today joins an empty `XDG_RUNTIME_DIR` (a relative `devdeck`, so
    `file.read` is refused in a cwd's `devdeck` folder); `runtime()` ignores an empty one.
    Taken: a fix on Linux, said in the commit.
13. Test holes. Taken: `time` with a POSIX `TZ` of a non-zero offset (`XXX-2`), set before
    the first local time call; `signals` raises SIGHUP only once the handle exists, and
    waits with a timeout; the fallback after windows close, a signal then quit (one
    kill), and the `accept` check of point 6 are tests too.
14. `docs/DESIGN.md:383` says `devdeck-api` "depends on nothing of devdeck". Taken: it
    depends on `devdeck-sys` for the socket's folder; the clients still depend only on
    `devdeck-api` (R13.11).
15. The Mac clippy check should build from Linux (`rustix`, `libc`, `signal-hook` have no
    C code on Apple) provided nothing pulls `devdeck-core` in and `unsupported.rs` has
    no unused parameter. Taken: run before the commit.

## Step 1a, as built (2026-11-10)

- `crates/devdeck-sys`: `lib.rs` (`SysError`, `Support`, `Capability` with `Runtime`,
  `Memory`, `OpenPath`, `support()`: constants per target so far), `dirs/` (`xdg.rs` for
  both OSes, `runtime` from `linux.rs` or `unsupported.rs`, `SOCKET_PATH_MAX` 107 on
  Linux, 103 elsewhere), `memory/`, `time/`, `fs/` (`try_lock`, `no_follow`, `is_mine`,
  `is_loop`, `open_path`, `make_fifo`), `signals/` (`QuitSignals`), `net/` (`shutdown`,
  `hung_up`), a `#[cfg(test)]` `TempDir` and the source scan (`scan.rs`, 11 files left
  to move, each with its step).
- Callers moved: core's three folder functions are one line each; `devdeck-api`'s
  `default_socket` returns the socket or why there is none, and `devdeck-ctl` and
  `devdeck-mcp` print that reason; `api::Config.runtime` is the socket folder or its
  reason (`api::no_runtime()` for tests and the snapshot tool); `socket::bind` takes the
  folder itself; `devdecks_own`, `read_file`, the transport, the instance lock, the
  signal thread, the clock and the stats go through `devdeck-sys`. `signal-hook` left
  `devdeck-core` (unused) and `devdeck-app`; `devdeck-app` keeps `rustix` and `libc` for
  `api/identity.rs` (1c).
- `font-kit` in `gpui_platform`'s features; `Cargo.lock` unchanged but for the new crate.
- The quit observer in `Shell::start`, guarded by `quit_for_real` and `killed` (set by
  `kill_everything`); `quit_now` with `quit_for_real` kills what the runs left when
  nothing did yet (diff review point 1); `kills` counts for tests.
- `logger.rs` in `devdeck-app`, installed by `main.rs` after the environment is scrubbed.
- `text_never_wraps_at_large_scale` is Linux only.
- Docs: `DESIGN.md` (the crate list, what `devdeck-api` depends on), `PLATFORMS.md` (§4's
  modules built; rows L6–L8, L14, U4–U6, U8, U11 point at `devdeck-sys`), `BUILDING.md`
  and `main.rs`'s header (`DEVDECK_LOG`).
- Checked: the whole suite and clippy on Linux; the Mac check (`cargo clippy --target
  aarch64-apple-darwin -p devdeck-sys -p devdeck-api -p devdeck-ctl -p devdeck-mcp
  --all-targets -- -D warnings`), clean. One start of the demo: the only line logged is
  GPUI's `WARN gpui::text_system: failed to load font … ".SystemUIFont"` (its default
  font; ours are bundled), silent until now. The Settings view's "API off" text is the
  same as before on Linux (`XDG_RUNTIME_DIR isn't set`), so no snapshot.
- Found, not of this step: a devdeck stopped by a signal (SIGTERM from `timeout`) leaves
  its socket and lock in the runtime folder; the build from before this step does the
  same.

### Step 1a diff review (subagent), assessed

1. `quit_now` set `killed` without killing, so a ctrl+q with nothing running left a
   build's daemon in its scope and the observer skipped it. Taken: `killed` only in
   `kill_everything`; `quit_now` (for real) kills when nothing did yet. Tested through
   "Stop all and quit" (one kill, none at GPUI's quit after); the path with nothing
   running isn't, since the test workspace's demo runs always go.
2. `eprintln!` in the logger panics when stderr is gone. Taken: `writeln!`, error ignored.
3. The quit tests counted kills only and missed `quit_now`; the window-closed fallback in
   `main.rs` is untested. Taken for "Stop all and quit" (a test). The fallback stays in the
   binary: moving it is step 4's (L13), with point 5.
4. The scan: per-file allowance, no manifest check, missed patterns. Partly taken: its
   header now says the manifests guard the rest only after 1d. Not taken: `/run/` (the
   hits are sample data in the snapshot tool and a test, not OS calls); a count per file
   (the files listed are the ones being rewritten in 1b–1d).
5. On macOS the window-closed fallback might run before GPUI's quit observers and kill
   without marking runs to start again. Not taken now: whether AppKit closes windows
   first is for round 0 to show; step 4 designs the last window (L13).
6. Every quit that skips the shell (stress mode's) now kills again after the loop ends.
   Not taken: harmless, nothing real runs in the demo, and it is the observer's job.
7. `default_socket`'s test lost "DEVDECK_SOCKET wins". Taken.
8. `a_socket_path_fits_its_address` tested nothing. Taken: dropped; `socket.rs` tests that
   a path past `SOCKET_PATH_MAX` is refused.
9. A non-UTF-8 `XDG_RUNTIME_DIR` now reads as unset. Not taken: the reason says "isn't
   set" where it is unusable, and such a folder breaks GPUI before.
10. `TZ` is set safely only under nextest. Taken: the SAFETY comment says so. The other
    timing points stand as reviewed (the `accept` test is for round 0 to judge on macOS).
11. `fs`, `net` and `signals` include `unix.rs` on every OS, and `support()` isn't used
    yet. Not taken: no third OS is built; `support()` gets its users with the actions
    that need it.
12. Docs and comments: `PLATFORMS.md` L6 and L12, `shell.rs`'s header, `lifecycle.rs`'s,
    the `fs` typo, a code span split in `socket.rs`, `DEVDECK_LOG`'s `error` and `off`.
    Taken.

## Step 1b plan (2026-11-10): the process table and ports

Every read of the process table (L3) and of listening ports (L4) moves into
`devdeck-sys`, with no change on Linux. On macOS both are unsupported until 2b (the
process table) and 2d (ports). R11.6, R6.9, R8.2.

### `procs/` (L3)

From `devdeck-core/src/procfs.rs` and the `/proc` reads around it.

| Function | Replaces |
|---|---|
| `get(pid) -> Option<Proc>`: parent, session, start, whether it ended (a zombie) | `procfs::parent_of`, `parent_and_start`; `process.rs`'s `stat` and `parse_stat` (`:561-575`) |
| `parents() -> Result<Vec<(u32, u32)>, SysError>` | `procfs::parents` |
| `name(pid) -> Option<String>` | `procfs::name`; `process.rs:249` (a shell tab's foreground program) |
| `command(pid) -> Option<String>`: the command line, arguments joined | `ports.rs`' `command_of`; `lifecycle.rs:200-204`'s copy |
| `alive(pid, start)`: running, and the same process as before | `process.rs:588` |
| `exists(pid)` | `devdeck-core/src/testing.rs:63` (the sweep), `tests/app/pty_driver.rs:130` |
| `tree(root, pairs)`, `descendants(root)` | `procfs::tree`, `descendants` (plain Rust, `mod.rs`) |

- The start is an opaque number: equal for the same process only. On Linux it stays
  clock ticks since boot, so the group log already on disk stays valid.
- `procfs.rs` is deleted; its parsers and their tests move to `procs/linux.rs`.
- Callers: `process.rs` (`collect`, `kill_all`, `leftovers`, `foreground_program`, its
  tests' `session_of`), `port_scan.rs:282-283,325`, `api/mod.rs:228`,
  `api/identity.rs:45-46`, `tests/app/lifecycle.rs:41,49` (`Orphan`).

### `ports/` (L4)

- `listening(pids) -> Result<Vec<Listening>, SysError>`, `holder(port) ->
  Result<Option<Holder>, SysError>`, `Listening`, `Holder`, from
  `devdeck-core/src/ports.rs`. `/proc/net/tcp*` parsing, the socket links and the
  whole-table scan go to `ports/linux.rs`; `Holder::Process` takes its command line from
  `procs::command`.
- `PortMemory` (the ports each command used last time) isn't OS code: it stays in core's
  `ports.rs`, alone there.
- Callers import from `devdeck_sys::ports`: `port_scan.rs:17`, `tests/app/ports.rs:10`.
- The parse tests move to `linux.rs`; the tests with real listeners (IPv4, IPv6, pids
  not scanned, who holds a port) become conformance tests in `mod.rs`.

### Unsupported (decided by the user, 2026-11-10)

The reads over the whole table (`parents`, `listening`, `holder`) return `Result`, as
`docs/PLATFORMS.md` §4 asks: on a Mac "no ports" must not read as "none found". Answers
about one pid (`get`, `name`, `command`) stay `Option`: "can't read that one" is a
normal answer already. Callers keep today's behaviour on an error, so nothing changes on
Linux; `port_scan.rs` logs the reason once (`log::warn!`), so round 0's log says ports
aren't found on this OS yet. Showing reasons in the window comes with step 4.

### The scan's list

Off it after 1b: `procfs.rs` (gone), `ports.rs`, `testing.rs`, `lifecycle.rs`,
`tests/app/pty_driver.rs`, `tests/app/lifecycle.rs`. Left: `process.rs`, `scope.rs`,
`git.rs`, `environment.rs` (1d), `api/identity.rs` (1c). `process.rs` stays for its
rustix calls; it no longer reads `/proc` itself.

### Tests

- Conformance in `procs/mod.rs`: this process's parent readable and started before it; a
  spawned `sh -c 'sleep 5'` in this process's tree; its `name` and `command`; once killed
  and reaped, `exists` and `alive` false, its start no longer matching.
- Conformance in `ports/mod.rs`: the real-socket tests above.
- macOS: each function's unsupported answer, under `#[cfg(not(target_os = "linux"))]`,
  compiled by the Mac check.
- The whole suite and clippy on Linux, the Mac check, the scan with its shorter list.

### States to check visually

None change on Linux: the Ports activity, a port conflict before a start, the leftovers
dialog. No snapshot.

### Out of scope

The API's callers (1c); starting and stopping runs (1d); the macOS process table (2b)
and ports (2d); reasons shown in the window (step 4).

### Step 1b plan review (subagent), assessed

Where a point is taken, it overrides the plan above. Point 1 was checked again in
`api/identity.rs:56-68`, point 3 in `port_scan.rs:64-70`.

1. The start isn't only opaque: `identity::chain` keeps a parent only while it started no
   later than its child. Taken: the start is ordered, a later process has a larger
   number; on Linux clock ticks since boot as today. 2b's backend must keep that order.
2. `exists` and `alive` as plain bools can only say "gone" on a Mac, and the test sweep
   would then delete the folders of tests still running. Taken: `exists` goes in a shared
   `unix.rs` with `kill(pid, 0)` (gone only on `ESRCH`; `EPERM` means it exists), "any
   process with this pid, a zombie included", as `/proc/<pid>` says today; so it works on
   macOS from 1b. `alive` stays on `get` (unsupported reads as not alive until 2b, which
   only the leftovers dialog uses).
3. The two `command_of` differ: the ports' falls back to the name, the leftovers' doesn't.
   Taken: `command` is the command line only (`None` when unreadable or empty);
   `ports/linux.rs` falls back to `name`, `lifecycle.rs` doesn't, so neither text changes.
4. Tests moved to `ports/mod.rs` and the procs conformance tests would fail on a Mac, or,
   gated one by one, leave dead helpers that fail the Mac clippy. Taken: the real-socket
   and real-process tests, their helpers and imports, go in one `#[cfg(target_os =
   "linux")] mod conformance` per module until 2b/2d; no `mod.rs` helper only `linux.rs`
   uses. `command.contains("devdeck_core")` becomes `devdeck_sys`.
5. Test holes. Taken: spawn `sleep 5` itself (`sh -c` may exec it, so the name would
   race); a zombie test (killed, not reaped: `exists` true, `alive` false; reaped: `exists`
   false); `alive(pid, wrong start)` false; `stat_fields_survive_odd_command_names`
   (`process.rs:1122-1127`) joins the parse tests and checks the session too; the short
   `stat` lines in `ppid_survives_odd_command_names` get the fields a full parse needs,
   `(a) b (c)` kept; "its start no longer matching" after reaping dropped (`get` is `None`).
6. Linux mustn't fail where it answers today. Taken: `holder` keeps `Ok(Some(Unknown))`
   when `/proc` can't be listed; a missing `/proc/net/tcp6` (IPv6 off) is skipped as today;
   on Linux `Err` only when `/proc/net/tcp` itself can't be read.
7. The call sites of the `Result` change weren't listed. Taken: `holder` at
   `port_scan.rs:187,302,409` and `tests/app/ports.rs:132`; `listening` at
   `port_scan.rs:288`; `parents` at `process.rs:478,758` and `port_scan.rs:282`; each keeps
   today's answer on `Err`. `descendants` (only a test uses it) leaves the API: the test
   takes `tree(root, &parents()?)`. `use devdeck_core::procfs` at `port_scan.rs:18` goes.
8. "Log once" needs pinning. Taken: a `static` `Once` in `port_scan.rs`; it logs the first
   `Err` of `Job::run` (`listening`, `holder`, `parents`), nothing else; core
   (`process.rs`, no `log`) stays silent.
9. "Ended" must cover `X` (dead) as well as `Z`. Taken. `tests/app/lifecycle.rs`'s
   `Orphan::alive` (no start) maps to "`get` says not ended".
10. No `SUPPORT` or `Capability` for the new modules. Taken: `Capability::Processes` and
    `Ports`, a `SUPPORT` per module, as 1a's.
11. Docs: PLATFORMS §1 L3 and L4 and §4's "built so far"; `port_scan.rs:3,7`,
    `process.rs:8,565`, `identity.rs`' header name `procfs` or `core::ports`. Taken. §4's
    Mac check lists `-p devdeck-core`, which only compiles for macOS after 1d: taken, §4
    says so. `scripts/measure.py` reads `/proc` by design (L11 lists it): not taken.

## Step 1b, as built (2026-11-10)

- `devdeck-sys/src/procs/`: `Proc` (parent, session, start, ended for `Z` and `X`),
  `get`, `parents` (a `Result`), `parent_of`, `parent_and_start`, `name`, `command`
  (`None` when empty), `alive`, `exists` (`unix.rs`, `kill(pid, 0)`, both OSes), `tree`;
  `linux.rs` reads `/proc`, `unsupported.rs` for macOS until 2b. `descendants` is gone:
  its one test takes `tree(root, &parents()?)`.
- `devdeck-sys/src/ports/`: `listening` and `holder` (both `Result`), `Listening`,
  `Holder`; `linux.rs` keeps today's reads (`tcp6` skipped when missing, `holder`
  `Unknown` when `/proc` can't be listed; `Err` only when `/proc/net/tcp` can't be
  read); `unsupported.rs` for macOS until 2d.
- `Capability::Processes` and `Ports`, with a `SUPPORT` each.
- `devdeck-core`: `procfs.rs` deleted; `ports.rs` keeps `PortMemory`; `process.rs` reads
  the table through `procs` (its own `stat` parser went to `procs/linux.rs`, its test
  with it, checking the session too); `testing.rs`' sweep uses `procs::exists`.
- `devdeck-app`: `port_scan.rs` imports from `devdeck_sys`, keeps today's answer on an
  error and logs the first one (`unsupported`, a `static Once`); `lifecycle.rs`,
  `api/mod.rs`, `api/identity.rs`, `tests/app/pty_driver.rs`, `tests/app/lifecycle.rs`
  and `tests/app/ports.rs` go through `procs` and `ports`.
- The scan's list: `process.rs`, `scope.rs`, `git.rs`, `environment.rs` (1d),
  `api/identity.rs` (1c).
- Docs: `PLATFORMS.md` L3, L4, §4's modules built and its Mac check (`-p devdeck-core`
  after 1d).
- Checked: the whole suite (821) and clippy on Linux, the Mac check, clean. No visible
  change on Linux, so no snapshot.
- The diff review by a subagent was refused by the session's permission check; the lead
  reviewed the diff against the old code instead (`process.rs`, `port_scan.rs`, the
  moved parsers): no behaviour change found on Linux; one doc line rewrapped.
