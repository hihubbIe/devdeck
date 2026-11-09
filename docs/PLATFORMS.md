# Platforms

devdeck is built and used on Linux. This file lists everything in it that is not
OS-generic, what macOS would use instead, and how OS code is organised so a feature
can be supported, partly supported or refused per OS. Inventory as of 2026-11-09
(commit `9f40aa0`), reviewed against the code and against how established Rust
projects do it (§6).

Status words:
- **No build**: the code does not compile for macOS.
- **Silent**: it compiles but finds nothing or does nothing on macOS.
- **Off**: the feature turns itself off on macOS.
- **Works**: it works on macOS, but not the way a Mac app should.
- **To verify on a Mac**: read from documentation or source, not yet seen on a Mac.

## 1. Linux only

| Id | Capability | Where | Linux | macOS equivalent | On macOS today |
|---|---|---|---|---|---|
| L1 | Waiting for a run to exit, waking its thread, reading its terminal | `devdeck-core/src/process.rs:143` (eventfd), `:190` (pidfd_open), `:342-351` (poll on the wake fd, the pidfd and the terminal), `:403` (waitid on a pidfd) | `eventfd`, `pidfd_open`, `poll`, `waitid(WaitId::PidFd)` | one `kqueue` carrying the terminal fd, a wake pipe and `EVFILT_PROC`/`NOTE_EXIT`, then `waitpid`. Apple's poll(2) says it "does not support devices", so `poll` on the terminal is not kept (to verify on a Mac; Alacritty and rio use kqueue there) | No build |
| L2 | A run's processes kept together and killed as one (R8.2) | `devdeck-core/src/scope.rs` (whole file), `process.rs:146-160`, `devdeck-app/src/main.rs:39` | `systemd-run --user --scope`, cgroup v2 and `cgroup.kill` under `/sys/fs/cgroup`, `/proc/self/cgroup`, `pidfd_open` + `pidfd_send_signal` | no cgroup, no child subreaper, and XNU refuses `NOTE_TRACK`. Best effort: own session, `killpg`, then walk the descendants (L3) and signal any that left the group. A process that double-forks and calls `setsid` is reparented to launchd and escapes unless an earlier walk saw it | No build (pidfd calls); the logic already turns off without systemd |
| L3 | Process table: parent, children, name, command line, start time, liveness | `devdeck-core/src/procfs.rs`, `process.rs:478,486,565,671,758`, `devdeck-app/src/lifecycle.rs:201`, `port_scan.rs:259,302`, `api/mod.rs:188`, `api/identity.rs:45-46` | `/proc/<pid>/stat`, `comm`, `cmdline`, listing `/proc` | `proc_listallpids`, `proc_pidinfo(PROC_PIDTBSDINFO)`, `proc_name`, `sysctl(KERN_PROCARGS2)` for the command line (what WezTerm's `procinfo` and Alacritty use) | Silent: stop reaches only the process group; leftovers of a crashed devdeck aren't found |
| L4 | Ports a run listens on (Ports activity) | `devdeck-core/src/ports.rs:75,81,124,129` | `/proc/net/tcp`, `tcp6`; socket inodes through `/proc/<pid>/fd` | `proc_pidinfo(PROC_PIDLISTFDS)`, then `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` per fd | Silent: no ports found |
| L5 | Who calls the control API (R13) | `devdeck-app/src/api/identity.rs` (whole file) | `SO_PEERCRED` (`rustix::net::sockopt::socket_peercred`, Linux only), `SO_PEERPIDFD` (hard-coded 77, `identity.rs:15`, which is wrong on some architectures such as mips and sparc), `/proc/self/fdinfo/<fd>`, poll on the pidfd | `getpeereid` for the user; `LOCAL_PEERTOKEN` for an audit token whose (pid, pidversion) pins the process against pid reuse, as a pidfd does (macOS 13+ per Apple DTS); `LOCAL_PEERPID` as a fallback. rustix has none of these; `nix` has all three | No build |
| L6 | Where the control API's socket lives | `devdeck-app/src/api/mod.rs:79,196`, `main.rs:82`, `api/socket.rs:12-13` (`MAX_PATH = 107`), `devdeck-api/src/client.rs:14-19`, `devdeck-ctl/src/main.rs:130`, `devdeck-mcp/src/lib.rs:131` | `$XDG_RUNTIME_DIR/devdeck/`; `sun_path` holds 108 bytes | no runtime dir; `sun_path` holds 104 bytes. `$TMPDIR` is long and its unused files are purged after about 3 days, so it is not used (§5) | Off: control API, `devdeck-ctl` and `devdeck-mcp` don't work |
| L7 | Config, state and notes folders | `devdeck-core/src/settings.rs:61-67`, `state.rs:3,19`, `notes.rs:14,19`, `config.rs:3` | XDG base dirs, else `~/.config`, `~/.local/state`, `~/.local/share` | see §5 | Works |
| L8 | devdeck's own memory, for stats | `devdeck-app/src/stats.rs:19-28` | `/proc/self/smaps_rollup` | `task_info(TASK_VM_INFO)`, `phys_footprint` | Silent |
| L9 | Window frame | `devdeck-app/src/window_view.rs:127-155,702-819,1004` | client-side decorations: our own title bar and buttons; `app_id` matched by the `.desktop` launcher | the system's traffic lights; ours would duplicate them. gpui has `TitlebarOptions { appears_transparent, traffic_light_position }` | Works, looks foreign |
| L10 | Shortcuts | `devdeck-app/src/registry.rs` (every binding), `devdeck-ui/src/terminal.rs:356-359`, `devdeck-ui/src/code.rs:159` (`EDITOR_KEYS`, a copy of gpui-component's Input bindings on Linux), `devdeck-ui/tests/ui/code.rs:184` (`OperatingSystem::Linux`) | `ctrl-…` | `cmd-…` for app keys through gpui's `secondary-` modifier; `ctrl-…` stays the terminal program's; cmd-c / cmd-v copy and paste in the terminal. Where gpui-component switches its Input bindings per OS is to verify | Works, feels wrong |
| L11 | Build, install, launcher | `scripts/install.sh`, `docs/BUILDING.md` | apt packages, Vulkan driver, `~/.local/bin`, `.desktop` file and hicolor icon | Xcode for `xcrun metal` (or gpui's `runtime_shaders` feature), a `.app` bundle with `Info.plist` and `.icns`, signing (§5) | Missing |
| L12 | Tests that read Linux state | `devdeck-app/tests/app/lifecycle.rs:41,49,113`, `tests/app/pty_driver.rs:130`, `devdeck-core/src/testing.rs:63`, tests in `ports.rs`, `process.rs:845,925,933`, `scope.rs` | `/proc`, `systemd-run`, `setsid` | through the OS layer's conformance tests and fakes (§4) | Fail |
| L13 | Menu bar, Dock, last window closed | none: no `set_menus`, no `on_reopen`; `devdeck-app/src/main.rs:44` stops everything when the last window closes | not expected on Linux | a menu bar (devdeck: About, Settings, Hide ⌘H, Quit ⌘Q; Window), a Dock click that reopens the window, and a choice whether closing the last window quits | Missing |

Not affected: GPUI has a macOS backend (`gpui-pre-macos`, Metal); the `wayland` and
`x11` features of `gpui_platform` only apply on Linux. `notify` uses FSEvents on
macOS. `kube`, `alacritty_terminal` and the tree-sitter grammars are portable.
`ioctl_tiocsctty`, `waitid`, `kill_process_group` and `IUTF8` exist on macOS in rustix.
The login-shell environment capture (`environment.rs:46-48`) already gives an app
started from Finder or the Dock the user's PATH, as Zed does. Fonts are bundled.

## 2. Unix: Linux and macOS, not Windows

These work on both, except where marked. A Windows port, or the move into the OS
layer (§4), starts from this list.

| Id | Capability | Where | Note |
|---|---|---|---|
| U1 | Unix domain sockets (control API) | `devdeck-app/src/api/socket.rs`, `api/transport.rs`, `api/handlers.rs:807-822`, `devdeck-api/src/client.rs:10,74` | path length: L6 |
| U2 | Pseudo-terminals | `devdeck-core/src/process.rs:287-305` (`openpt`, `grantpt`, `unlockpt`, `ptsname`, `tcsetwinsize`, `tcgetattr`, `ioctl_tiocsctty`) | **No build**: `OpenptFlags::CLOEXEC` (`process.rs:289`) exists only on Linux, FreeBSD and NetBSD (rustix 1.1.6 `pty.rs:45-46`); macOS sets `FD_CLOEXEC` with `fcntl` after `openpt` |
| U3 | Sessions, process groups, signals | `process.rs:166-170,241,516-551,751`, `git.rs:723-742,826-836`, `environment.rs:126-160` | |
| U4 | devdeck's own signals | `devdeck-app/src/lifecycle.rs:76-82` (`signal_hook`: SIGTERM, SIGHUP, SIGINT) | |
| U5 | File locks | `api/socket.rs:6,54`, `lifecycle.rs:39` (`flock`) | |
| U6 | Permission bits, owner, `O_NOFOLLOW`, `O_NONBLOCK` | `api/socket.rs:8,54,76`, `api/handlers.rs:807-822`, `environment.rs:103,225`, `apps.rs:250`, `toolchains/node/versions.rs:155`, `toolchains/project.rs:116` | |
| U7 | Shells | `/bin/sh` in `process.rs:159`, `environment.rs:47,351-371`, `scope.rs:105`; `$SHELL` for the login environment in `environment.rs:47-61` | |
| U8 | Local time | `devdeck-app/src/settings_view.rs:44-47` (`localtime_r`) | |
| U9 | Byte paths | `devdeck-app/src/drops.rs:73-75` (test, already `cfg(unix)`) | |
| U10 | `$HOME` | `devdeck-core/src/config.rs:232`, `discovery/mod.rs:58`, `environment.rs:61`, `devdeck-app/src/deck.rs:216`, `api/handlers.rs:773`, `devdeck-kube/src/kubeconfigs.rs:265` | |

## 3. Requirements and decisions shaped by Linux

To revisit when the macOS port starts:

- **R1.2** says "Linux first (Wayland and X11)": it gains macOS.
- **R8.2** (children never outlive the app) is exact on Linux with scopes, best effort
  on macOS (L2). The requirement says so.
- **R9.6** (a scroll gesture moves one axis) is described from Wayland's events; its
  wording and tests are checked against macOS trackpads.
- **R3.8**: tabs aren't dragged between windows (GPUI can't do it on Wayland); "Send
  to…" replaces it. On macOS it could be revisited.
- Window positions are never kept (`devdeck-core/src/layout/mod.rs:162`) because
  Wayland doesn't let apps place windows. On macOS they could be.
- APFS is case-insensitive by default: path comparisons in discovery, files and notes
  are audited (not done yet).
- On a Mac without the Command Line Tools, `/usr/bin/git` is a stub that opens an
  install dialog: the git panel says git is missing instead of starting it.

## 4. The OS layer

### Shape

One crate, `devdeck-sys`, is the only code that touches the OS directly. Every other
crate asks it, in devdeck's terms: "the processes under this pid", "the ports these
pids listen on", "who is on this socket", not "read `/proc/net/tcp`". This is the
layout of Rust's std (`library/std/src/sys/<capability>/`) and of WezTerm's
`procinfo` and `portable-pty`: cfg-selected modules inside one crate for syscall-sized
capabilities. Separate crates per OS, as gpui has, are for something as large as a
windowing backend.

It is split by capability, not by OS:

```
crates/devdeck-sys/src/
  lib.rs            Capability, Support, support(), SysError
  child/            L1, L2, U2, U3: spawn in a terminal, read and write it, wait,
                    wake, stop and kill the tree (one event loop per OS)
  procs/            L3: the process table
  ports/            L4
  peer/             L5: who is on a socket, pinned against pid reuse where the OS can
  dirs/             L6, L7: config, state, data and runtime folders; socket path limit
  memory/           L8
  signals/          U4: devdeck's own SIGTERM/SIGHUP/SIGINT
  fs/               U5, U6: locks, permissions, no-follow opens
  time/             U8
```

Each module:

```
  procs/
    mod.rs          the shape: types and functions devdeck calls, documented once;
                    the conformance tests
    linux.rs
    macos.rs
    unsupported.rs  every function returns Unsupported
```

`mod.rs` picks one backend from the target being built:

```rust
#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(target_os = "macos", path = "macos.rs")]
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), path = "unsupported.rs")]
mod imp;

/// `root` and every process below it.
pub fn tree(root: u32) -> Result<BTreeSet<u32>, SysError> {
    imp::tree(root)
}
```

The build flag is the target itself (`cargo build --target aarch64-apple-darwin`);
nothing is chosen by hand. The public function in `mod.rs` fixes the shape: a backend
with a different signature doesn't compile. Code shared by Linux and macOS (most of
U2–U6) lives in a `unix.rs` both backends call, so a Windows backend is one more file
per module.

Not in `devdeck-sys`: shortcuts and window chrome. They are UI and stay in
`devdeck-app` / `devdeck-ui`. Shortcuts use gpui's `secondary-` modifier (cmd on macOS,
ctrl elsewhere), with per-OS keymaps where a key differs, as Zed ships
`default-linux.json` and `default-macos.json`. The title bar picks its
`TitlebarOptions` per OS (L9). The menu bar and Dock (L13) are app code too.

### Supported, partly, or not

- `support(Capability) -> Support`, one of `Yes`, `Partial(why)`, `No(why)`. It is
  computed once at startup and cached, because some support is only known at
  runtime: systemd present (L2), kernel 6.5 or later for `SO_PEERPIDFD` (L5),
  `$XDG_RUNTIME_DIR` set (L6).
- A function that can't work returns `Err(SysError::Unsupported(why))`, never an empty
  answer that reads as "nothing found". This is std's choice
  (`io::ErrorKind::Unsupported`).
- The action registry asks `support()`: an action whose capability is `No` is shown
  disabled, with the reason, in the palette and in menus, so mouse and keyboard agree
  (R5.1). `Partial` is shown where it matters, e.g. "stop is best effort on macOS"
  (L2).

### Testing

- **Conformance tests** in each `mod.rs` run against whichever backend is compiled,
  on every OS in CI: spawn `sh` in a terminal, read its output, find its tree, find a
  port it opens, stop it, identify a peer on a socket pair.
- **Fakes** for the app: `devdeck-sys` exposes each capability through a trait seam
  that tests can replace, so the app's tests stop reading `/proc` (L12) and run the
  same on every OS.

### Keeping it there

- **Dependencies are the guard.** `rustix`, `libc`, `nix` and `signal-hook` are
  dependencies of `devdeck-sys` only; every other crate drops them from its
  `Cargo.toml`, so it cannot call the OS even by accident. `nix` is a macOS-only,
  target-gated dependency (L5).
- **A source scan test** fails when `/proc`, `/sys/`, `XDG_RUNTIME_DIR`, `systemd-run`
  or `std::os::linux` appears in a source file outside `devdeck-sys`: literals and paths
  that no dependency rule catches.
- **CI** (none exists yet; it comes before the first macOS backend):
  - Linux: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
  - Linux: `cargo check -p devdeck-sys --target aarch64-apple-darwin`, a fast check
    that the macOS backend compiles. Also usable locally once the target is added
    with `rustup target add`. `-p` is right here, unlike elsewhere (`docs/WORKFLOW.md`):
    the rest of the workspace does not build for macOS yet, and another target has its
    own folder in `target/`.
  - macOS (`macos-15`, pinned; `macos-latest` moved to macOS 26): `cargo clippy
    --workspace --all-targets -- -D warnings` and `cargo test -p devdeck-sys`.
- **Build scripts**: `devdeck-sys` has no build script, and no dependency of it needs
  Apple's SDK to build, so the Linux cross-check keeps working. The whole workspace
  can't be cross-checked from Linux: GPUI embeds a Metal shader library compiled by
  `xcrun metal` unless its `runtime_shaders` feature is on.

### Build or buy

| Capability | Choice | Why |
|---|---|---|
| L1, U2, U3 child | build, on rustix | rustix has kqueue with `EVFILT_PROC`; no crate does terminal plus exit together better. `portable-pty` adds nix and anyhow and doesn't wait for exit |
| L2 tree kill | build | `process-wrap` only does groups and sessions, which rustix already gives |
| L3 process table | build, on libc | devdeck needs a narrow slice; WezTerm's `procinfo` does it in about 150 lines. `sysinfo` would work but is heavy |
| L4 ports | build on libc; `listeners` as an alternative | to decide once `libc`'s `proc_pidfdinfo` socket structs are checked; `netstat2` needs bindgen |
| L5 peer | `nix` (macOS only) | it has `getpeereid`, `LocalPeerPid`, `LocalPeerToken`; `interprocess` gives no pid on macOS |
| L6, L7 dirs | build (keep ours) | small; `dirs` has no runtime dir on macOS and puts config in `~/Library` |
| L8 memory | build, on libc | one call |
| U1 sockets | keep std's `UnixListener` | |
| U4 signals | keep `signal-hook` | |

### Every feature from now on

A plan says whether the step touches the OS. If it does, the step goes through
`devdeck-sys`, with a Linux backend, a macOS backend, or an explicit `Unsupported`
with its reason, lists which in the plan, and adds the capability's conformance test.
The review checks it.

## 5. Decisions

Recommended from the review (§6). Each is the user's to confirm.

| Topic | Recommendation |
|---|---|
| Crate name | `devdeck-sys` |
| Existing Unix code (§2) | moves into `devdeck-sys` with the port, U2/U3 together with L1/L2 in `child` |
| L7 folders on macOS | config in `~/.config/devdeck` (what Zed, WezTerm, Alacritty, Helix do for config), state and notes in `~/Library/Application Support/devdeck`, logs in `~/Library/Logs/devdeck` |
| L6 socket on macOS | `~/Library/Application Support/devdeck/run/` (0700) when its path fits in 104 bytes, else `/tmp/devdeck-<uid>/` (0700, owner checked, as tmux does). Not `$TMPDIR`. `devdeck-ctl` and `devdeck-mcp` resolve it through the same `devdeck-sys` function |
| L9 window | transparent title bar with our content beside the traffic lights, as Zed does |
| L13 last window | native per OS: on Linux, closing the last window quits and stops every run; on macOS, devdeck and its runs keep going, a Dock click reopens the window as it was, and ⌘Q or Quit in the menu quits and stops everything (R8.2) |
| L11 builds | a GitHub Actions `macos-15` runner builds the `.app` (Zed's `script/bundle-mac` is the model: bundle, `codesign`, DMG, notarize, staple) |
| L11 signing | ad-hoc for sharing with a few people: on current macOS they open it once through System Settings → Privacy & Security → Open Anyway (Control-click Open no longer works). Developer ID and notarization only for wider distribution |
| Process rule | done: R11.6, `WORKFLOW.md` (plan, review, definition of done), `CLAUDE.md` |

To verify on a Mac before relying on it:
- `poll` on a terminal fd (L1): kept out of the design either way.
- Where gpui-component switches its Input bindings per OS (L10).
- `libc`'s coverage of the `proc_pidfdinfo` socket structs (L4).
- The macOS version that introduced `LOCAL_PEERTOKEN` (L5); devdeck falls back to
  `LOCAL_PEERPID` without it.
- How Zed places its title bar content beside the traffic lights (L9).

## 6. Sources

Crate sources read under `~/.cargo/registry/src/`: rustix 1.1.6 (`pty.rs`,
`net/sockopt.rs`, `event/kqueue.rs`, `process/mod.rs`, `event/mod.rs`), gpui-pre 0.3.11
(`platform/keystroke.rs`, `platform.rs`), gpui-pre-platform 0.3.11, gpui-pre-apple 0.3.11
(`build.rs`), alacritty_terminal 0.26.0 (`tty/unix.rs`), mio 1.2.4, tokio 1.54.1
(`process/unix/`), dirs 6.0.0 (`mac.rs`), nix 0.31.3 (`sys/socket/sockopt.rs`).

- Rust std's per-capability `sys` layout: https://github.com/rust-lang/rust/issues/117276
- Apple poll(2), BUGS: https://developer.apple.com/library/prerelease/ios/documentation/System/Conceptual/ManPages_iPhoneOS/man2/poll.2.html
- libvirt, poll on devices on macOS: https://listman.redhat.com/archives/libvir-list/2020-October/209960.html
- XNU (`bsd/sys/un.h`, `bsd/kern/kern_event.c`): https://github.com/apple-oss-distributions/xnu
- Apple DTS on `LOCAL_PEERTOKEN`: https://developer.apple.com/forums/thread/850777
- `$TMPDIR` cleanup: https://developer.apple.com/forums/thread/71382
- libproc-rs build script: https://github.com/andrewdavidmackenzie/libproc-rs/blob/master/build.rs
- Config locations: https://ghostty.org/docs/config, https://wezterm.org/config/files.html,
  https://alacritty.org/config-alacritty.html
- Docker Desktop socket: https://docs.docker.com/desktop/setup/install/mac-permission-requirements/
- GitHub runner images: https://github.blog/changelog/2026-05-14-github-actions-upcoming-image-migrations
- Signing on Apple Silicon: https://eclecticlight.co/2020/08/22/apple-silicon-macs-will-require-signed-code/
