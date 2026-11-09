# Building devdeck

## Toolchain

Rust stable (developed with 1.99). `rustup update stable`.

## Linux system packages

Debian / Ubuntu:

```sh
sudo apt install pkg-config libfontconfig-dev libfreetype-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libx11-dev libx11-xcb-dev libxcb1-dev \
  libvulkan-dev mesa-vulkan-drivers
```

A working Vulkan driver is needed at runtime (GPUI renders through wgpu).

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

GPUI needs the desktop session's `XDG_RUNTIME_DIR` (and `WAYLAND_DISPLAY` or `DISPLAY`).
A shell that lacks it (one in a tmux server started outside the session, for instance)
panics with "XDG_RUNTIME_DIR is not set". A fallback in `~/.bashrc` fixes it:

```sh
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
```

## Install

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

## Tests and lints

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

From the workspace root, with `--workspace`; one area's tests by name (`cargo test
--workspace --test app deck::`), never with `-p` (`docs/WORKFLOW.md`, "One way to build").

## The build folder

`target/` is a cache: everything in it is rebuilt by cargo, and the installed binaries
are copies (`scripts/install.sh`), so `cargo clean` never breaks anything. Each kind of
build keeps one set there, rebuilt in place by the same command. Measured 2026-11-09,
cargo 1.99, from a clean folder:

| Build | Command | Adds |
|---|---|---|
| tests | `cargo test --workspace` | 6.2 GB (12 test binaries, peak memory 5.2 GB cold) |
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
