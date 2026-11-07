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

## Tests and lints

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
