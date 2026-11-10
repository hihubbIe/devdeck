# Building devdeck

How to build and run devdeck from source, one section per platform. Each section is
complete on its own: read only the one for your system.

**Contents**

- [Supported platforms](#supported-platforms)
- [macOS](#macos)
- [Linux](#linux)

The app is called **Kugelblitz**; its program, files and folders are named `devdeck`.

## Supported platforms

| Platform | Status |
|---|---|
| Linux, x86_64 (developed on Ubuntu, Wayland and X11) | Supported. |
| macOS 15 or later, Apple silicon | Builds from source; not yet confirmed on a Mac. Not supported on macOS yet: starting real commands and terminals, the ports list, and the control API (which lets `devdeck-ctl` and `devdeck-mcp` talk to a running devdeck). Keys are Linux's (ctrl, not ⌘); no menu bar or Dock integration yet. |

Intel Macs and Windows are not supported.

## macOS

### 1. Requirements

An Apple silicon Mac with macOS 15 or later, and about 10 GB of free disk space: the
release build takes about 2.6 GB, the tests' build about 6.2 GB, and building the tests
uses up to about 5 GB of memory (measured on Linux). The first build takes a while.

```bash
uname -m
```

prints `arm64`.

```bash
sw_vers -productVersion
```

prints `15` or a higher number first.

```bash
df -h ~
```

shows the free space in the column "Avail".

### 2. Xcode

devdeck's interface library, GPUI, compiles its graphics shaders with Xcode's `metal`
tool, which comes only with the full Xcode app, not with the Command Line Tools alone.
Source: [Zed, Building Zed for macOS](https://zed.dev/docs/development/macos) (GPUI is
Zed's).

1. Install Xcode from the App Store.
2. Open it once and install the macOS components it offers (the default choice).
3. Install the command line tools:

   ```bash
   xcode-select --install
   ```

   A dialog opens: choose Install and wait until it finishes. If the tools are already
   there, the command says so instead.
4. Point the command line tools at Xcode, and accept its licence:

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

prints `/Applications/Xcode.app/Contents/Developer`.

```bash
xcrun --find metal
```

prints a path ending in `/metal`. If it says `unable to find utility "metal"` instead,
the Metal toolchain is a separate download (Zed's guide notes this from macOS 26):

```bash
xcodebuild -downloadComponent MetalToolchain
```

If that fails, run `xcodebuild -runFirstLaunch`, then the download again.

### 3. Rust

From [rust-lang.org](https://rust-lang.org/tools/install/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

It asks how to install: the default is right. To skip the question, use `sh -s -- -y`
instead of `sh` at the end
([rustup book](https://rust-lang.github.io/rustup/installation/index.html)). Then open a
new terminal, or run in this one the line the installer printed at its end that loads
`.cargo/env`.

Check:

```bash
rustc --version
```

prints `rustc 1.99.0` or later (devdeck is developed with 1.99).

### 4. nextest

Only needed to run the tests (step 8). nextest is the test runner devdeck's tests use.
From [nexte.st](https://nexte.st/docs/installation/pre-built-binaries/) (one binary for
every Mac):

```bash
curl -LsSf https://get.nexte.st/latest/mac | tar zxf - -C ${CARGO_HOME:-~/.cargo}/bin
```

Check:

```bash
cargo nextest --version
```

prints `cargo-nextest 0.9.…` (developed with 0.9.151).

### 5. Get the source

`git` comes with Xcode.

```bash
git clone https://github.com/hihubbIe/devdeck.git
```

```bash
cd devdeck
```

Every command below runs from this folder.

### 6. Build

```bash
cargo build --release
```

Builds `devdeck`, `devdeck-ctl` and `devdeck-mcp` into `target/release/`. Its last line
starts with `Finished`.

### 7. Run

```bash
target/release/devdeck
```

A window called Kugelblitz opens. The first time, it shows "No workspace yet": devdeck
shows the projects of the folders a workspace file names. Make the folder for workspace
files:

```bash
mkdir -p ~/.config/devdeck/workspaces
```

and create a file in it, for example `~/.config/devdeck/workspaces/work.toml`, naming a
folder that holds your projects (`~/code` here):

```toml
name = "work"
folders = ["~/code"]
```

Each folder inside it is then one project, its commands read from files such as
`package.json`, `go.mod`, `Cargo.toml`, a `Makefile` or a docker compose file. The
welcome screen shows more options. Restart devdeck after creating the first file; from
then on it follows changes to the files and the folders. A folder inside Documents or
Desktop makes macOS ask whether Terminal may access it: allow it, or devdeck can't list
it.

`target/release/devdeck --demo` shows a demo instead: sample workspaces with simulated
commands, kept apart from your own settings and layouts.

Quit with ctrl+Q, or close the window. Closing the terminal that started devdeck quits it
too.

### 8. Run the tests

```bash
cargo nextest run --workspace --no-fail-fast
```

Builds the tests (about 6.2 GB), then runs them all, even after a failure. The tests of
what macOS doesn't support yet fail on a Mac: those whose names contain `pty_driver::`,
`terminals::` or `ports::`, some of `lifecycle::`,
`files_only_inside_the_workspaces_folders` and `every_editor_key_is_known`, and possibly
a few socket and file watcher tests.

Optional: macOS checks every new program for malware (XProtect), which slows each test.
Adding Terminal to Developer Tools exempts what it starts
([nexte.st, macOS](https://nexte.st/docs/installation/macos/)): run `sudo spctl
developer-mode enable-terminal`, then in System Settings, Privacy & Security, Developer
Tools, turn Terminal on, and restart it. This changes a security setting.

### 9. Update

Quit devdeck first.

```bash
git pull
```

```bash
cargo build --release
```

After updating Rust (`rustup update`), run `cargo clean` before building: a new compiler
rebuilds everything and would leave the old build in `target/`, taking space.

### 10. devdeck's files

| What | Where |
|---|---|
| Settings; workspace files (`workspaces/*.toml`); paths of this machine such as Node folders and PATH additions (`machine.toml`) | `~/.config/devdeck` (or `$XDG_CONFIG_HOME/devdeck`) |
| Layouts, pins, history (the demo keeps its own in `demo/`) | `~/.local/state/devdeck` (or `$XDG_STATE_HOME/devdeck`) |
| Notes | `~/.local/share/devdeck` (or `$XDG_DATA_HOME/devdeck`) |

`DEVDECK_CONFIG_DIR`, `DEVDECK_STATE_DIR` and `DEVDECK_DATA_DIR` replace these folders.
To start fresh, quit devdeck and delete `~/.local/state/devdeck`: settings, workspace
files and notes stay.

### 11. Uninstall

Delete the cloned `devdeck` folder and, to remove settings, state and notes, the folders
above. To remove Rust too: `rustup self uninstall`
([rust-lang.org](https://rust-lang.org/tools/install/)).

### 12. Troubleshooting

| Problem | Fix |
|---|---|
| `xcrun: error: unable to find utility "metal"` | Step 2: `sudo xcode-select --switch …`, then the Metal toolchain. |
| The build stops on Xcode's licence | `sudo xcodebuild -license accept` |
| `command not found: cargo` | Open a new terminal (step 3). |
| "dependency is using unstable features" | `cargo clean`, then build again. |
| `Too many open files (os error 24)` in the tests | Run them with nextest (step 8), not `cargo test`. |
| devdeck lists no project from a folder in Documents or Desktop | System Settings, Privacy & Security, Files and Folders: allow Terminal. |
| A crash shows no function names | The release build keeps none. Build a debug one, `cargo build` (about 1.9 GB more), and run `RUST_BACKTRACE=1 target/debug/devdeck`. |

devdeck writes warnings and errors to the terminal that started it. For more detail,
start it with `DEVDECK_LOG=info` (or `debug`, `trace`); `DEVDECK_LOG=off` silences it.

## Linux

### 1. Requirements

Linux on x86_64, with a desktop session (Wayland or X11) and a working Vulkan driver
(GPUI, devdeck's interface library, draws with the GPU), and about 10 GB of free disk
space: the release build takes about 2.6 GB, the tests' build about 6.2 GB, and building
the tests uses up to about 5 GB of memory. The first build takes a while.

```bash
uname -m
```

prints `x86_64`.

```bash
df -h ~
```

shows the free space in the column "Avail".

### 2. System packages

A C compiler, `git`, `curl`, and the libraries GPUI uses. Debian and Ubuntu names:

```bash
sudo apt install build-essential git curl pkg-config libfontconfig-dev libfreetype-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-dev libx11-xcb-dev \
  libxcb1-dev libvulkan-dev mesa-vulkan-drivers
```

Other distributions: the same under their own names.

### 3. Rust

From [rust-lang.org](https://rust-lang.org/tools/install/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

It asks how to install: the default is right. To skip the question, use `sh -s -- -y`
instead of `sh` at the end
([rustup book](https://rust-lang.github.io/rustup/installation/index.html)). Then open a
new terminal, or run in this one the line the installer printed at its end that loads
`.cargo/env`.

Check:

```bash
rustc --version
```

prints `rustc 1.99.0` or later (devdeck is developed with 1.99).

### 4. nextest

Only needed to run the tests (step 9). nextest is the test runner devdeck's tests use.
From [nexte.st](https://nexte.st/docs/installation/pre-built-binaries/):

```bash
curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C ${CARGO_HOME:-~/.cargo}/bin
```

Or from source: `cargo install cargo-nextest --locked`
([nexte.st](https://nexte.st/docs/installation/from-source/)).

Check:

```bash
cargo nextest --version
```

prints `cargo-nextest 0.9.…` (developed with 0.9.151).

### 5. Get the source

```bash
git clone https://github.com/hihubbIe/devdeck.git
```

```bash
cd devdeck
```

Every command below runs from this folder.

### 6. Build

```bash
cargo build --release
```

Builds `devdeck`, `devdeck-ctl` and `devdeck-mcp` into `target/release/`. Its last line
starts with `Finished`.

### 7. Run

```bash
target/release/devdeck
```

A window called Kugelblitz opens. The first time, it shows "No workspace yet": devdeck
shows the projects of the folders a workspace file names. Make the folder for workspace
files:

```bash
mkdir -p ~/.config/devdeck/workspaces
```

and create a file in it, for example `~/.config/devdeck/workspaces/work.toml`, naming a
folder that holds your projects (`~/code` here):

```toml
name = "work"
folders = ["~/code"]
```

Each folder inside it is then one project, its commands read from files such as
`package.json`, `go.mod`, `Cargo.toml`, a `Makefile` or a docker compose file. The
welcome screen shows more options. Restart devdeck after creating the first file; from
then on it follows changes to the files and the folders.

`target/release/devdeck --demo` shows a demo instead: sample workspaces with simulated
commands, kept apart from your own settings and layouts.

Quit with ctrl+Q, or close the window. Closing the terminal that started devdeck quits it
too.

### 8. Install

```bash
scripts/install.sh
```

Builds the release binaries and installs them for this user: `devdeck`, `devdeck-ctl`
and `devdeck-mcp` in `~/.local/bin`, devdeck's icon (a black hole), and a launcher in
`~/.local/share/applications` that the desktop's app grid shows as Kugelblitz. On
Ubuntu's desktop, press Super, type Kugelblitz, right-click it and choose Pin to Dash: a
click on it then brings the open window forward. Started from the launcher, devdeck has
the desktop session's environment, and the commands and terminals it starts get the
login shell's on top (PATH, nvm, pnpm).

### 9. Run the tests

```bash
cargo nextest run --workspace --no-fail-fast
```

Builds the tests (about 6.2 GB), then runs them all, even after a failure.

### 10. Update

Quit devdeck first, or restart it after: a running one keeps the old binary.

```bash
git pull
```

```bash
scripts/install.sh
```

Without the installer: `cargo build --release` instead. After updating Rust
(`rustup update`), run `cargo clean` before building: a new compiler rebuilds everything
and would leave the old build in `target/`, taking space.

### 11. devdeck's files

| What | Where |
|---|---|
| Settings; workspace files (`workspaces/*.toml`); paths of this machine such as Node folders and PATH additions (`machine.toml`) | `~/.config/devdeck` (or `$XDG_CONFIG_HOME/devdeck`) |
| Layouts, pins, history (the demo keeps its own in `demo/`) | `~/.local/state/devdeck` (or `$XDG_STATE_HOME/devdeck`) |
| Notes | `~/.local/share/devdeck` (or `$XDG_DATA_HOME/devdeck`) |
| The control API's socket | `$XDG_RUNTIME_DIR/devdeck` |

`DEVDECK_CONFIG_DIR`, `DEVDECK_STATE_DIR` and `DEVDECK_DATA_DIR` replace the first three.
To start fresh, quit devdeck and delete `~/.local/state/devdeck`: settings, workspace
files and notes stay.

### 12. Uninstall

```bash
scripts/install.sh --uninstall
```

Removes what step 8 installed. Then delete the cloned `devdeck` folder and, to remove
settings, state and notes, the folders above. To remove Rust too: `rustup self
uninstall` ([rust-lang.org](https://rust-lang.org/tools/install/)).

### 13. Troubleshooting

| Problem | Fix |
|---|---|
| `linker 'cc' not found` | Install the C compiler: `sudo apt install build-essential` (step 2). |
| devdeck stops at start with an error naming `XDG_RUNTIME_DIR` or the Wayland compositor | The shell lacks the desktop session's environment (a tmux server started outside the session, for instance). Set it in the shell's startup file (`~/.bashrc` for bash): `export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"` |
| No window, or an error about Vulkan or the GPU | Install the Vulkan driver for the GPU (`mesa-vulkan-drivers` for AMD and Intel). |
| `command not found: cargo` | Open a new terminal (step 3). |
| "dependency is using unstable features" | `cargo clean`, then build again. |
| A crash shows no function names | The release build keeps none. Build a debug one, `cargo build` (about 1.9 GB more), and run `RUST_BACKTRACE=1 target/debug/devdeck`. |

devdeck writes warnings and errors to the terminal that started it. For more detail,
start it with `DEVDECK_LOG=info` (or `debug`, `trace`); `DEVDECK_LOG=off` silences it.
