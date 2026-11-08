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
cargo run --release -p devdeck-app
```

That runs the `devdeck` binary (the package's `default-run`); the snapshot tool needs
`--bin devdeck-snapshot --features snapshot`.

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
icon (`assets/app/devdeck.svg`) and a launcher in `~/.local/share/applications`. Then
press Super, type devdeck, right-click it and choose Pin to Dash: the dock keeps it,
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
