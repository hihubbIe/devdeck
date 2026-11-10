# Building devdeck

Two setups, each whole on its own: [macOS](#macos) and [Linux](#linux). What follows them
is shared: building and running, tests and lints, the build folder.

## macOS

> **Draft, not yet run on a Mac.** Round 0 of the macOS milestone
> (`docs/plans/macos.md`) is its first run; what it finds goes back in here.

This section is written for someone who doesn't know Rust, following it with a coding
agent. It goes from a fresh Mac to a running devdeck and a file of test results. What to
try once devdeck runs, and what to send back, is in
[`docs/plans/macos-manual-test.md`](plans/macos-manual-test.md), "Round 0".

Steps marked **(you)** open a window or ask for a password: the tester does them, not the
agent. Read steps 1 to 3 on GitHub; from step 4 on, the agent works inside the clone.

**Rules for the agent**

- This section and the round's notes are all you need; the rest of `docs/` is for the
  developer.
- Change nothing: no edits to tracked files, no commit, no push, no `cargo update`,
  `cargo fmt`, `cargo fix`, `cargo clippy --fix` or `cargo add`. `git status` is clean
  when the results are sent.
- A build or test failure is not fixed here: save the output and send it back. Fixes are
  made by the developer, where the Linux suite and lints run.
- Install nothing this section doesn't name (no Homebrew packages).
- Cargo runs from the repository's root, always with `--locked` (`Cargo.lock` stays as
  it is), tests with `--workspace`; never `-p`. Never `scripts/install.sh` (Linux only).
- macOS has no `timeout` command: long commands are left to finish.

### 1. The Mac (you or the agent)

devdeck needs Apple silicon and macOS 15 or later.

```bash
uname -m
```

Prints `arm64`. Anything else (`x86_64`): an Intel Mac, not supported.

```bash
sw_vers -productVersion
```

Prints `15.` something or a higher number.

```bash
df -h ~
```

About 15 GB free under "Avail". Measured on Linux: the release build 2.6 GB, the tests'
build 6.2 GB (peak memory 5.2 GB), a debug build 1.9 GB (only after a crash). Time on a
Mac is unmeasured; the first build is long.

### 2. Xcode (you)

devdeck's interface library (GPUI) compiles its graphics shaders with Xcode's `metal`
tool, which only the full Xcode has: the Command Line Tools alone are not enough. Source:
[Zed, Building Zed for macOS](https://zed.dev/docs/development/macos) (GPUI comes from
Zed).

1. Install Xcode from the App Store.
2. Launch it once and install the macOS components it offers (the default option).
3. In Terminal:

```bash
xcode-select --install
```

It may answer that the tools are already installed: fine.

```bash
sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
```

```bash
sudo xcodebuild -license accept
```

Check:

```bash
xcode-select -p
```

Prints `/Applications/Xcode.app/Contents/Developer`.

```bash
xcrun --find metal
```

Prints a path ending in `/metal`. If it says `unable to find utility "metal"`, on macOS 26
or later:

```bash
xcodebuild -downloadComponent MetalToolchain
```

and if that fails, `xcodebuild -runFirstLaunch`, then the download again (Zed's guide).

Note Xcode's version for the results: Xcode menu, About Xcode.

### 3. The code (you or the agent)

`git` comes with Xcode.

```bash
git clone https://github.com/hihubbIe/devdeck.git
```

```bash
cd devdeck
```

Check: `git log -1 --oneline` prints one commit.

Now start the coding agent **inside this folder**: it reads the repository's `CLAUDE.md`,
which sends it here.

### 4. Rust (the agent)

From [rust-lang.org](https://rust-lang.org/tools/install/); `-y` takes the defaults
without asking, as the [rustup book](https://rust-lang.github.io/rustup/installation/index.html)
shows (`sh -s -- … -y`):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

The installer adds Rust to new shells only. To use it in the same shell, run the line it
printed at its end that loads `.cargo/env`; or **(you)** open a new terminal in the
folder.

```bash
rustc --version
```

Prints `rustc 1.99.0` or later (devdeck is developed with 1.99). `command not found`: the
line above wasn't run, or open a new terminal.

### 5. nextest, the test runner (the agent)

From [nexte.st, pre-built binaries](https://nexte.st/docs/installation/pre-built-binaries/)
(one binary for both Mac kinds):

```bash
curl -LsSf https://get.nexte.st/latest/mac | tar zxf - -C ${CARGO_HOME:-~/.cargo}/bin
```

```bash
cargo nextest --version
```

Prints `cargo-nextest 0.9.…` (developed with 0.9.151).

Optional **(you)**: macOS scans every new program for malware (XProtect), which slows
each test; adding Terminal to Developer Tools exempts what it starts
([nexte.st, macOS](https://nexte.st/docs/installation/macos/)). It changes a security
setting, so it is the tester's choice: `sudo spctl developer-mode enable-terminal`, then
System Settings, Privacy & Security, Developer Tools: Terminal on; restart Terminal.

### 6. Build (the agent)

```bash
cargo build --release --locked 2>&1 | tee ~/devdeck-build.log
```

Builds `devdeck`, `devdeck-ctl` and `devdeck-mcp` into `target/release/`. Its last line
starts with `Finished`. An error: stop here and send `~/devdeck-build.log` with the
versions (the round's notes).

### 7. Run (you)

```bash
target/release/devdeck 2>&1 | tee ~/devdeck-run.log
```

A window opens, called **Kugelblitz** (devdeck's name as an app). The terminal stays busy
while it runs; its log goes to the file. Quit with **ctrl+Q** (Control, not ⌘: Mac keys
come later) or by closing the window. Closing the Terminal window quits devdeck too.

`target/release/devdeck --demo` shows the demo: sample workspaces and simulated runs.

What to try, and what to expect: the round's notes.

If devdeck crashes ("panicked" in the log): the release build has no function names, so
build the debug one (1.9 GB more) and send its log:

```bash
cargo build --locked
```

```bash
RUST_BACKTRACE=1 target/debug/devdeck 2>&1 | tee ~/devdeck-debug.log
```

### 8. Tests (the agent)

```bash
cargo nextest run --workspace --locked --no-fail-fast > ~/devdeck-tests.log 2>&1
```

It builds the tests first (6.2 GB), then runs them all, even after failures. Some fail
in this round on purpose: the notes list them. Check: the file ends with a `Summary`
line. A test still marked `SLOW` after 5 minutes: stop with ctrl+C, write down its name,
send the file as it is.

### 9. Update (the agent)

```bash
git pull
```

Then steps 6 to 8 again. After a `rustup update`, `cargo clean` first: a new compiler
rebuilds everything and leaves the old build behind ("The build folder").

### 10. devdeck's files on a Mac

| What | Where (this round) |
|---|---|
| Settings and workspace files | `~/.config/devdeck` (`workspaces/<name>.toml`) |
| Layouts, pins, history | `~/.local/state/devdeck` (the demo: `…/devdeck/demo`) |
| Notes | `~/.local/share/devdeck` |

From the macOS milestone's step 2a, layouts and notes move to
`~/Library/Application Support/devdeck`; what this round left in the two folders above
isn't carried over. Settings and workspace files stay in `~/.config/devdeck`.

Start fresh (quit devdeck first): delete `~/.local/state/devdeck`; notes and workspace
files stay. Remove everything: the three folders, the `devdeck` clone, and Rust with
`rustup self uninstall` ([rust-lang.org](https://rust-lang.org/tools/install/)).

### Troubleshooting

From Zed's guide and nextest's, until round 0 adds its own.

| Error | Fix |
|---|---|
| `xcrun: error: unable to find utility "metal"` | step 2: `xcode-select --switch`, then the Metal toolchain |
| a licence not accepted | `sudo xcodebuild -license accept` |
| `command not found: cargo` | step 4: the `.cargo/env` line, or a new terminal |
| "dependency is using unstable features" | `cargo clean`, then build again |
| `Too many open files (os error 24)` | run tests with nextest (step 8), not `cargo test` |
| devdeck ends at start with `killed: 9`, or macOS says it is damaged | send it back with the log |
| macOS asks for access to Documents or Desktop | allow it, or devdeck may list no apps from there; if refused, System Settings, Privacy & Security, Files and Folders |

## Linux

### Toolchain

Rust stable (developed with 1.99). `rustup update stable`.

Tests run with [cargo-nextest](https://nexte.st) (developed with 0.9.151). Its pre-built
binary for Linux x86_64, as its install page gives it
([nexte.st/docs/installation/pre-built-binaries](https://nexte.st/docs/installation/pre-built-binaries/#linux-x86_64)):

```sh
curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C ${CARGO_HOME:-~/.cargo}/bin
```

Or from source: `cargo install cargo-nextest --locked`
([nexte.st/docs/installation/from-source](https://nexte.st/docs/installation/from-source/)).

### System packages

Debian / Ubuntu:

```sh
sudo apt install pkg-config libfontconfig-dev libfreetype-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libx11-dev libx11-xcb-dev libxcb1-dev \
  libvulkan-dev mesa-vulkan-drivers
```

A working Vulkan driver is needed at runtime (GPUI renders through wgpu).

### The session's environment

GPUI needs the desktop session's `XDG_RUNTIME_DIR` (and `WAYLAND_DISPLAY` or `DISPLAY`).
A shell that lacks it (one in a tmux server started outside the session, for instance)
panics with "XDG_RUNTIME_DIR is not set". A fallback in `~/.bashrc` fixes it:

```sh
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
```

### Install

```sh
scripts/install.sh
```

Builds the release binary and installs it for this user: `~/.local/bin/devdeck`, its
icon (`assets/app/devdeck.svg`, a black hole) and a launcher in
`~/.local/share/applications`. The installed app is called **Kugelblitz**; the binary,
the window's app id and the files stay `devdeck`. Then press Super, type Kugelblitz,
right-click it and choose Pin to Dash: the dock keeps it,
and a click on it brings the open window forward (the launcher's `StartupWMClass` is the
window's app id, `devdeck`). Run the script again to update (quit devdeck first or
restart it after: a running one keeps the old binary). `scripts/install.sh --uninstall`
removes the three files; settings and state stay.

Started from the dock, devdeck has the desktop session's environment, and runs and
terminals get the login shell's on top (PATH, nvm, pnpm).

## Build and run

```sh
cargo build --release
target/release/devdeck
```

The same release build `scripts/install.sh` makes. `cargo run --release` would make a
second one: `cargo run` selects only the binary's package, and another selection resolves
shared dependencies with other features ("The build folder"). For a debug run, `cargo build`
then `target/debug/devdeck` (`cargo build`'s set; `cargo run` would keep one of its own);
the snapshot tool: `cargo run --bin devdeck-snapshot --features devdeck-app/snapshot`
(a set of its own anyway: its features change GPUI's).

devdeck logs warnings and errors on stderr, its own and GPUI's, one line each (level,
where from, message). `DEVDECK_LOG=info`, `debug` or `trace` shows more, `error` less,
`off` nothing.

## Tests and lints

```sh
cargo nextest run --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

From the workspace root, with `--workspace`; one area's tests by name (`cargo nextest run
--workspace deck::`), never with `-p` (`docs/WORKFLOW.md`, "One way to build"). Nextest
runs no doctests; no crate has any (`doctest = false`). `cargo test --workspace` still
works, on the same build, running the binaries one after another.

## The build folder

`target/` is a cache: everything in it is rebuilt by cargo, and the installed binaries
are copies (`scripts/install.sh`), so `cargo clean` never breaks anything. Each kind of
build keeps one set there, rebuilt in place by the same command. Measured 2026-11-09,
cargo 1.99, from a clean folder:

| Build | Command | Adds |
|---|---|---|
| tests | `cargo nextest run --workspace` (or `cargo test`, the same build) | 6.2 GB (12 test binaries, peak memory 5.2 GB cold) |
| check and clippy | `cargo clippy --workspace --all-targets` | 1.4 GB |
| debug binaries | `cargo build` | 1.9 GB |
| debug run, snapshot tool, release | `cargo run …`, `cargo build --release` | one set each |

Before (2026-11-09 morning): a test binary per test file with full debug info, about
1.1 GB each, and one full set per `-p` selection: 435 GB in three days, and builds that
took more than 50 GB of memory.

What is left behind: a dependency change (`Cargo.lock`) rebuilds the crates above it
under new names and keeps the old ones (one bump of `itoa`: 42 crates, 3.5 GB), and a new
rustc rebuilds everything. Cargo removes nothing itself (its `-Z gc` is nightly only), so
`target/` is cleared at the end of every milestone and after every `rustup update`
(`docs/WORKFLOW.md`), or whenever it grows past about 30 GB.

Debug builds keep line tables only, split out of the binaries (`Cargo.toml`): backtraces
name file and line; a debugger shows the stack, not variables. For variables, `cargo
build --profile dbg` (full debug info, its own folder `target/dbg`, deleted when done).
