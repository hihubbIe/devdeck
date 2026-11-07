# M2 · Real local runner — plan

Goal: devdeck runs the user's real apps from config: apps found in the workspace
folders, tools and Node versions detected, each run in a pseudo-terminal shown in a real
terminal view, env applied, ports detected, nothing left running after quit. The demo
catalog stays for tests, snapshots and the stress run (`--demo`, its own state folder).

Requirements: R1.4, R1.5, R4.4 (terminal exception), R6.1–R6.9, R8.1, R8.2, R9.3
(terminal component), R10.1, R11.1–R11.5.

## Decisions (user, 2026-11-07)

- A terminal wraps at the pane width and never scrolls horizontally (R4.4 exception).
- Each run starts with a fresh terminal. Run output keeps the program's own colors;
  coloring by level stays in the logs view.
- In a terminal, keys go to the process except `alt+…` and `ctrl+shift+…`.
- R6.1 adds Go (`go.mod` with a main package) and Cargo.
- Config changes apply on save; runs of an app that disappears keep running and stay
  visible.
- Env binding: a command-line argument is preferred (nothing to write to disk). Bindings
  are stored generically (a `{env}` placeholder in a command line, a variable name, or
  later a file template) and added per app as needed, starting with a subset.
- Workspaces: `tadaweb` = `~/Documents/repos`; `personal` = `~/Documents/private_repos`
  and `~/Documents/public_repos` (in the user's config, step 5).
- A plain shell terminal tab is part of M2 (step 4). The rest of the user's ideas (file
  explorer, text editor, tools for Claude to drive devdeck) is a later milestone.

## Steps

1. **Foundations** (lead, serial; the demo stays green).
   - Run engine driven by driver events: a driver spawns and returns a handle (write,
     resize, stop); it reports bytes, the exit (code or signal) and spawn failures. The
     state machine follows the real exit; an exit caused by our stop doesn't toast.
   - One terminal model per run (alacritty_terminal, GPUI-free, in `devdeck-core`),
     owned by the runner so output is kept while no view shows it.
   - The fake driver writes ANSI bytes through the same path.
   - The run view draws the terminal (display only; input, selection and polish are
     builder F). Tests, snapshots and the stress run are ported.
   - Terminal trial: CPU and memory at 2 000 lines/s; go/no-go, and the scrollback cap
     comes from the measured memory.
   - Data model: envs belong to the workspace (not the cluster); app ids come from the
     folder path; command ids carry their source (`task:run`, `npm:dev`, `go:run`);
     apps get a working folder, a library flag and a Node requirement; `pre<x>`/`post<x>`
     are hooks only when `<x>` is a sibling script.
   - Libraries pinned; module interfaces for step 2 with every `mod` line and dependency
     in place, so builders touch only their own files.
   - The demo uses its own state folder.
2. **Builders in parallel**, each in its own git worktree with its own target folder,
   testing `devdeck-core` only (no GPUI build), except F.
   - A config: workspace files (folders, envs, overrides) and machine settings (Node
     folders, PATH additions) kept apart; unknown keys are errors; merging overrides into
     discovered apps.
   - B discovery: package.json and pnpm/npm workspaces, Taskfile, Makefile, docker
     compose, `.vscode/launch.json`, Go, Cargo; precedence (a Taskfile task wins over a
     bare `go run main.go`); skips `node_modules`; fixture trees.
   - C environment: the login shell's environment captured once; PATH lookup; `.nvmrc`
     searched upward with nvm aliases; pnpm availability per Node version; reasons.
   - D processes: a pseudo-terminal opened with rustix (own session and process group, exact environment; alacritty's `tty` passes devdeck's own environment on); stop sends ^C,
     then SIGTERM, then SIGKILL, with longer waits for compose; descendants that left
     the group are killed too; output drained before the exit is reported; process
     groups recorded so leftovers can be killed at the next start; SIGTERM, SIGHUP and
     SIGINT to devdeck stop the runs.
   - E ports: listening sockets of our runs' processes only; holders of other users;
     IPv6; ports remembered per command so conflicts show before a start.
   - F terminal element (main checkout, needs GPUI): drawing, key encoding with the
     pass-through rule, selection and copy, resize with the pane.
   Ownership during step 2: each builder owns only its file(s) in `devdeck-core`
   (`config.rs`, `discovery/`, `environment.rs`, `process.rs`, `ports.rs`); F owns
   `core/terminal.rs`, `ui/terminal.rs` and the run view. Plug points already in place:
   `Tools::toolbox` (C), `StopKind::of` (D), `environment::nvmrc` (done, for B),
   `procfs` (done, for D and E).
3. **Wiring** (lead): discovery before the window opens (restored tabs keep working);
   services from config; the real driver; folder watches (one per folder, not
   recursive, debounced) and catalog swap; ports scanned in the background; first-run
   screen without config; every quit path stops the runs.
4. **Terminal tab**: a shell in a folder, on the terminal element.
5. **Close**: the user's config, every state checked, measurements (CPU and memory with
   real output, `kill -9` and SIGTERM of devdeck then a leftover check), manual test.

## Step 1 outcome (2026-11-07)

Terminal trial: go (numbers in `docs/perf/M2.md`); history capped at 2 000 lines per run.
Review of the step 1 diff (subagent) and assessment:
- Stop didn't cancel a pending restart; a deferred restart could fail silently; the fake
  rate was wrong below ~63 lines/s; titles showed app ids. **Fixed.**
- Interface gaps (per-app tools, stop kind, new commands in overrides, port memory keys,
  B needing C's `.nvmrc` search, what `Launch` carries). **Fixed** in the interfaces.
- The emulator's colour and size queries get no reply; cursor and terminal modes aren't
  exposed. **Builder F.**
- Quit stops waiting after 5 s, before the stop sequence ends. **Builder D and step 3**:
  wait `StopKind::total()`, then `process::kill_all`.
- The catalog lacks the toolbox, port memory, problems and watch folders; the real
  driver merges the run's environment. **Step 3** (lead).
- Two views on one run resize it to the last one measured. **Builder F**, low.

## Step 2 outcome (2026-11-07)

Six builders, merged (`bb2298f`) plus the terminal display. Terminal drawing CPU 20.4 % →
~11 % (`docs/perf/M2.md`). Departures from the plan: processes use a pseudo-terminal
opened with rustix (alacritty's `tty` passes devdeck's environment on); machine paths
live in `machine.toml`, a file only the user writes (devdeck rewrites `settings.toml`).
Review of the combined step (subagent) and assessment:
- **Fixed**: launch.json variables in a fixed order (serde_json's order changes with the
  build's features); `**` repeats made globbing combinatorial (security review); group
  signals only while the group is ours (pid reuse); a dropped process is cleaned up off
  the UI thread, scanning before killing; `GroupLog::live` for signal-time kills; the
  login shell's output kept when a background job holds it open; Node folders default to
  nvm's; a comment-only `.nvmrc` doesn't stop the search; `//go:build ignore` files
  aren't mains; overrides recompute the library flag; keys after a run ended are the
  run view's again; terminal colour/size answers follow the theme before any run view
  paints; a hidden view stops sizing a run; a restart resets scrolling; a timing test
  made robust.
- **Step 3**: quit waits for the stop sequence (`StopKind::total`) before exiting, then
  `kill_all(GroupLog::live())`; SIGTERM/SIGHUP/SIGINT through signal-hook's thread; one
  devdeck at a time (another instance's runs would look like leftovers); the toolbox
  loads off the UI thread; a run that listened on nothing must not erase remembered
  ports; a broken port memory file is set aside silently.
- **Step 5**: descendants that left the group and lost their parent before a stop
  (subreaper or terminal-holder scan); groups whose leader ended but members run, after
  a crash; measure discovery on the real folders.
- **Accepted**: a port held on `::1` counts as a conflict for `127.0.0.1`; quitting from
  a focused terminal goes through the palette or the window button.

## Step 3 plan (2026-11-07)

1. Backbone (lead): a workspace comes from the demo or from a config file; `Services`
   builds from config (discovery, overrides, envs, problems, folders to watch) with no
   cluster until M3; the login shell's environment loads once, off the UI thread (apps
   show "checking tools…" until then) and reaches every workspace through a catalog
   swap, the same swap rediscovery uses; `devdeck --demo` runs the demo (its own state);
   without any config, a welcome view says where the files go. Problems are one toast
   with a count and a View that lists them.
2. Builders in parallel, each on its own new file: R1 real runs (`pty_driver.rs`), R2
   watching (`watcher.rs`), R3 lifecycle (`lifecycle.rs`: quit waits for the stop
   sequence then kills what's left, signals to devdeck, one devdeck at a time, leftovers
   offered at start), R4 ports (background scan, port memory, conflict before a start
   with "Stop holder").
3. Restore on reopen (user, 2026-11-07): every view comes back in its place, fresh;
   terminal contents aren't kept; a run that was running at quit starts again in its
   tab; views keep their own small state (scroll, follow, filters) next to the layout.
4. The user's config (moved here from step 5): `tadaweb` = `~/Documents/repos`,
   `personal` = `~/Documents/private_repos` and `~/Documents/public_repos`; real apps
   checked on screen; review; commit.

Decisions: a second devdeck says one is running and exits; leftovers after a crash are
offered in a dialog at start; problems are one toast with a count.

## Step 3 outcome (2026-11-07)

R1 and R4 by builders, R2 and R3 by the lead (the builders were refused). Discovery on
the real folders: `tadaweb` 172 apps in 88 ms, `personal` 19 apps in 13 ms. Review of
the step (subagent) and assessment:
- **Fixed**:
  - A rescan read every folder on the UI thread for any file event, logs and build
    files included. Now only a file discovery reads (`discovery::reads`), a new or
    renamed folder, or a removed watched folder rescans; the rescan runs in the
    background, swaps the catalog only when it changed, and the watcher follows its new
    folders afterwards.
  - A signal could let a killed run's exit reach the app first, which then saved it as
    not running. Now the app marks the runs stopped to quit, kills and saves; the signal
    thread kills only afterwards (or after 1 s if the app is stuck).
  - Leftovers the user left were killed at the next quit; Kill now re-checks each
    process's start time.
  - Config errors stayed in the problems view after a fix, and a broken `machine.toml`
    silently fell back to defaults. Now the config files' problems are read again on
    each change; a broken `machine.toml` keeps the current settings, with a toast; the
    last environment load asked wins.
  - View states (a run running) were saved only with a layout change or at quit; now
    soon after they change.
  - The quit wait used the current catalog's command, so a run whose command changed or
    disappeared got the default wait; each run now keeps its stop length.
  - A scan that saw only some of a run's ports replaced the remembered ones; a run's
    ports seen since it started are all kept.
  - A tab built while its app or command was missing stayed a message; it's built again
    when the apps change, its saved state kept.
  - The restart toast came back on every edit while a workspace file was broken; it now
    shows once per change of the set of files.
  - A folder that can't be watched (inotify limit) is reported once.
  - Smaller: a test's `sleep` always reaped; builder scaffolding removed from module
    docs; a typed check for tools still loading; the welcome text says a first workspace
    file needs a restart.
- **Assessed, not changed**: the conflict check before a start reads `/proc/net/tcp*`
  only (cheap); it scans every process's sockets only when a port is held, once per
  start, to name the holder. The first discovery stays on the UI thread (88 ms): the
  views restored with the window need their apps.
- **Added (user)**: the Apps list is a tree that keeps the folders.
- **Commands that work (user feedback after trying real apps)**:
  - Projects inside plain folders are found, down to 3 levels (`vhult/graph`,
    `tangine/*`).
  - Cargo: `cargo run` was offered for workspaces with several binaries, where cargo
    can't choose; now one command per binary, with `-p` when two packages share a name
    and the features a binary requires.
  - Node: a script's programs that aren't in any `node_modules/.bin` above the app and
    not on the PATH make the command say "vite isn't installed: run npm install"; the
    project gets an install command; an install made elsewhere is seen (its
    `node_modules/.bin` is watched). The list's ⚠ ignores the install command.
  - `engines.node` chooses the Node when there's no `.nvmrc`: the default when it
    fits, else the newest install that does.
  - Audit on the real workspaces: `personal` 35 apps in 2–22 ms, 125 of 134 visible
    commands ready (9 not installed). `tadaweb` 449 apps (272 in `dupes/`, two copies of
    `front-end`) in 160–230 ms, 2048 of 2844 ready: 782 not installed, 14 on Node 14.
    The first discovery is on the UI thread: 160–230 ms at start now.
  - Not checkable without running them: Makefile and Taskfile targets' own needs,
    compose (the docker daemon), launch.json's pre-launch tasks.

## Toolchains and command arguments: plan (user, 2026-11-07)

After the user tried real apps: everything about one language or toolchain lives in its
own module behind one generic interface, so adding one (Python later) is a new folder
and one line in a list. Today Node is spread over discovery, `environment.rs`
(installs, aliases, `.nvmrc`, `NODE_TOOLS`), `apps.rs` (`App::node`, `App::engines`)
and the discovery cache; Go's precedence rule sits in the generic app assembly.

**Part A: toolchains** (`devdeck-core/src/toolchains/`)

- `mod.rs`: the `Toolchain` trait and `ALL`, the list of toolchains; nothing
  language-specific. A toolchain:
  - `sources()`: the command sources it owns (`npm`, `pnpm`, `yarn`; `cargo`; `go`…),
    so a command finds its toolchain from its id.
  - `reads(name)` and `concerns(path)`: which changes can change what it finds (the
    watcher asks every toolchain; Node also watches `node_modules/.bin`).
  - `scan(dir, listing, parent, cx)`: its part of a project folder: commands, kind,
    whether it has something to run, the shell text its commands run, the facts it
    keeps about the app, and package globs for a monorepo root (the generic walk
    expands them and scans each package with the same toolchain, `parent` being the
    root's facts).
  - `covered(runs)`: its commands another toolchain already runs (Go: `go:run` when a
    Taskfile runs the main); default none.
  - `front(app, machine)`: folders to put first on a run's PATH (Node: the app's Node),
    or why it can't run; default none.
  - `installed(app, cmd, path)`: why the project isn't installed for this command
    ("vite isn't installed: run pnpm install"); default fine.
- Per-toolchain data in one place each: `Facts` (on `App`, one optional field per
  toolchain that keeps facts: Node's `.nvmrc`, `engines.node`) and `Machine` (in the
  toolbox: Node's installs, nvm aliases, the default Node). `App::node` and
  `App::engines` go.
- `shell.rs`: reading a script into the programs it runs (quotes, redirections,
  keywords, builtins), shared by toolchains.
- `node/`: `mod.rs` (package.json, manager, scripts with their deps, install command,
  monorepo globs), `versions.rs` (installs, aliases, `.nvmrc`, choosing by
  `engines.node`), `semver.rs` (npm ranges; an unreadable range lets the default Node
  run), `installed.rs` (what `node_modules/.bin` from the package up provides).
- `cargo/`: a manifest model (root package, members, default members, each package's
  binaries) read from the files, then a pure function from the model to run commands:
  bare `cargo run` runs what cargo picks by itself (one binary among the default
  packages, or `default-run`), every other binary gets `cargo:run:<bin>`, with `-p`
  outside the default packages or when two packages share the name, and the features
  it requires; a `"."` member isn't counted twice.
- `go.rs`, `make.rs`, `taskfile.rs`, `compose.rs`, `vscode.rs`: moved as they are.
- `discovery/`: only the generic walk (workspace folders, nested folders, monorepo
  packages, duplicate ids, assembling an app from the toolchains' parts, folders to
  watch) and globbing. A per-pass memo in `Cx` (by type and folder) replaces the
  Node-specific cache.
- `environment.rs`: only the login shell's variables, the PATH and `which`. The tool
  check becomes generic: each toolchain with facts on the app gives its PATH front
  (the command's own toolchain failing makes the command refused), then the
  command's programs are looked up, then its toolchain's `installed`.
- Tests move with their code: each toolchain tests itself on fixture trees (the fixture
  helper moves to `testing`); `discovery/tests.rs` keeps the walk's own.
- App side: the tool check runs once per change of apps or tools (`Services` keeps why
  each app can't be used); the sidebar reads that instead of checking every app on
  every render. When the watcher starts watching new folders after a rescan, it
  rescans once more (an install that finished in between is seen).
- Dropped: the Yarn Plug'n'Play case (not used).

Review of the plan (subagent) and assessment, all **accepted**:
- The tool check keeps today's rule: a command is refused when it needs a program a
  toolchain provides (`provides`: Node's `node`, `npm`, `pnpm`…) and that toolchain's
  PATH front failed, whatever the command's source (VS Code launches, config tools).
  Programs are looked up on the run's real PATH (fronts, then the base): a change for
  non-Node programs, tested.
- `.nvmrc` counts for every app, not only those with a `package.json`: after the parts
  are assembled, each toolchain completes its facts for any app folder.
- Machine data has a load step from `machine.toml`; the `node_dirs` key stays.
- The walk asks the toolchains which folders to skip and what a monorepo package is;
  every toolchain scans a package (a package can have its own Taskfile), with the
  root's facts (Node's manager, `engines.node`). A command fixing the project
  (`install`) is marked as such, not recognized by name. `Command::deps` becomes a
  generic "programs the project's install provides". `ALL`'s order decides an app's
  kind. A command whose source no toolchain owns gets the generic check only.
- Shared project-reading types (`Listing`, `Part`, quoting, YAML) live in
  `toolchains/`; the walk's context (`Cx`) is shared with them.
- Adding a toolchain is a folder, a line in `ALL`, and fields in `Facts`/`Machine` (and
  a config key) when it keeps any: the compiler points at each.
- Cargo: the default packages are `default-members`, else the root package when there
  is one, else every member; `default-run` counts only with one default package;
  `--features` comes with `-p` unless the binary's package is the only default one;
  `workspace.exclude` is honoured.
- Order: the current work is committed first; then shared types and `shell.rs`; the
  trait and moved sources; facts and machine; the generic check (current tests kept
  green, then moved); Cargo; the app side. A contract test checks `ALL` (sources
  unique, `reads` covering each manifest).

Part A outcome: built as planned, with these notes.
- The diff review by a subagent was refused by the permission check; reviewed by the
  lead instead: no language names left in generic code outside tests.
- On the real workspaces, discovery gives the same results as before (`personal` 35
  apps, `tadaweb` 449 apps, 2844 visible commands, 2048 ready), except the intended
  Cargo changes: `game` → `cargo run` and `cargo run -p xtask --bin xtask`;
  `tangine-dino`'s second binary next to its `default-run`.
- A test checks every toolchain's contract; the extra rescan's test fails without it.
- Seen once under load: `ports::tests::a_real_ipv4_listener_is_found` (another test
  can take the freed port); unchanged code, left as is.

**Part B: arguments per command** (planned in detail after A, with snapshots, then a
GO): commands stay the real distinct ones; each command gets arguments, chosen in the
run tab from a dropdown of recent ones (per workspace, app and command, most recent
first) or typed new; restart and reopen keep the last; each toolchain decides how they
reach the program (`with_args`: `npm run dev -- --port 1`, `pnpm run dev --port 1`,
`cargo run --bin x -- --flag`); the tab header shows the full line. Suggestions read
from projects come later if recent ones aren't enough.

## Part B: arguments per command, detailed plan (2026-11-07)

- **Where:** the run tab's toolbar, between the command line and the state: an
  arguments field showing the current arguments (`args…` dim when none) with a ▾.
- **Choosing:** a click (or `ctrl+shift+a`, a key the terminal leaves to devdeck) opens
  a menu (the existing overlay menu): "recent", up to 10 recent arguments, most recent
  first, the current one checked; "no arguments"; then "New arguments…", which turns
  the field into a text input (Enter runs with them, Esc cancels). Picking an entry
  while the run runs restarts it with them; otherwise it starts it.
- **What runs:** the toolbar's line and the run's header (`── run 2 · …`) show the
  full line with the arguments. They are passed as typed (your quotes are kept), added
  by the command's toolchain (`with_args`): npm and cargo after `--`; pnpm, yarn, go,
  make, task, compose and launch configs at the end.
- **Kept:** the current arguments are part of the run (restart reuses them) and of the
  tab's saved state (reopen restores them). Recent arguments are saved per workspace,
  app and command in the state folder (`args.json`, like the port memory).
- **Not in B:** other ways to start (app view, palette) use the run's current
  arguments; suggestions read from projects (launch.json `args`, Taskfile variables)
  come later if recent ones aren't enough.
- **Built with:** `Toolchain::with_args` and its tests per toolchain; a core
  `ArgsMemory` (load, remember, cap, order) tested on its own; `Runner` keeping
  arguments per run (tested with the fake driver); the run view's field and menu
  (snapshots: closed, open with recent ones, typing new ones; mouse and key tests).

Part B outcome: built as planned. The field shrinks before the toolbar's actions in a
narrow pane (seen on the first snapshots: "follow" was pushed out), and its tooltip
shows the full arguments. Snapshots 77–79.

## Terminal tabs: plan (step 4, 2026-11-07)

A tab with the user's shell in a folder, on the run tab's terminal (keys, selection,
links, resizing, killing on quit, restore), not a second copy of it:
- **Address:** `ViewAddr::Terminal { n, dir }` → `term/<n>/<dir escaped>`; the folder is
  in the address, so a restored layout knows it. `n` numbers terminals per workspace.
- **A run of an ad-hoc command:** the runner keeps ad-hoc launches next to the
  catalog's (`Runner::open_terminal`): an app (the catalog's app whose folder it is,
  for its Node; else one made for the folder) and a command `shell:<n>` running the
  login shell (`$SHELL` of the toolbox, else `/bin/sh`) interactively. Everything
  that looks a command up finds it there; the toolbox's PATH applies as for runs.
- **What differs from a run:** no arguments field; no success or failure toast; it
  stops with SIGHUP (a new `StopKind::HangUp`: hang-up, then 2 s, SIGKILL), as a
  terminal closing does. Closing the tab ends the shell (runs of commands keep
  running when their tab closes). The quit guard counts a terminal only while its
  shell runs something (it has child processes).
- **Its tab:** title `terminal · ~/folder`; started when the view is built (opened or
  restored: a fresh shell in the same folder); when the shell exits the tab stays
  ("ended"), ⏎ starts a new one.
- **Opening it (until the menus of the next item):** a palette command and a key,
  "New terminal" (`ctrl+shift+t`, unbound today), in the focused pane: in the folder of
  the focused tab's app (app or run tab), else the workspace's first folder (the home
  folder in the demo).
- **Tests:** the address round-trips; the runner opens, stops (hang-up) and lists
  terminals; closing the tab ends the shell; a restored layout starts a fresh shell;
  the quit guard; a real shell in a pseudo-terminal (as the pty driver tests do).

Review of the plan (subagent) and assessment, all **accepted**:
- Terminals are keyed apart from apps (`RunKey { app: "", cmd: "term/<n>" }`): no
  running dot on an app, no port memory or port conflict for them, no success or
  failure toast (a shell exits with its last command's status).
- Closing the tab goes through the deck's own closing (`Deck::sync`), not the view's
  release, and stops the shell (hang-up). A shell that exits takes what it left running
  with it (no leftovers).
- The view starts its shell when built, once the tools are known, unless one runs; it
  doesn't use the run tab's restore. The tab opens with the terminal focused.
- The quit guard counts a terminal while something runs in its foreground (the
  terminal's foreground group isn't the shell); quitting still waits for every shell
  to end.
- The shell runs as `exec "${SHELL:-/bin/sh}" -i`: interactive, not a login shell (the
  environment already is the login shell's); `TMUX`, `VTE_VERSION`, `TERM_PROGRAM`
  removed. Its rc files may put another Node first: the app's Node is a start, not a
  promise.
- A new terminal's number is above every terminal of the layouts (the book included).
  A folder that's gone shows the tab ended, without a toast. The demo's terminals say
  they need a real workspace. Real-shell tests use temp `HOME` and `ENV`.

Rules decided with the user (2026-11-07):
- Closing a terminal tab ends its shell and what runs in it; so does closing a detached
  window holding terminals ("closing is closing"; reattach first to keep them).
  Switching layouts doesn't: a terminal of another layout keeps running, its output
  arriving, and shows as it was when its layout comes back. Moving a tab or detaching
  keeps it.
- Every shell of a workspace is in one list, the sidebar's **Terminals** view: its
  folder, its state (idle, the program running in it, ended), where it shows (here,
  another layout, or not shown); open (activated where it shows, else in the focused
  pane) or end it. A terminal no layout shows any more stays listed, running, until
  ended there or quitting: nothing is ended unseen, nothing is lost.
- Quitting asks while any shell runs something, shown or not.

Terminal tabs outcome: built with these rules. Review of the diff (subagent), all
**fixed**: a terminal listening on a port crashed the port scan; an ended terminal
came back when a layout still holding it was shown (ending now removes it from every
saved layout, and a tab only starts a shell the runner never ran: `exit` stays ended
until ⏎); the arguments key restarted a shell; a terminal's ports showed as a run;
quitting with only idle shells skipped their hang-up; terminal rows could be pinned.

## Open terminal here, with places (2026-11-07)

- **Submenus, generic (devdeck-ui):** a menu item can carry a submenu of other ways to
  do it. A click does the item's default; resting the pointer on it 300 ms, or →,
  opens the submenu beside its row (first item level with it); ↑↓ ⏎ act in it, ← or
  Esc close it (Esc then closes the menu). An open submenu stays while the pointer
  crosses other rows towards it; resting on another row for the delay switches. A key
  wins over a pending rest.
- **Places (`places.rs`):** every pane, named as "Send to…" lists them (`n · window ·
  active tab`, now shared), and "new window". `places::item` makes the item: click →
  its default place, submenu → any place; places a view already open can't go to are
  disabled ("already here", "already alone in its window"). After opening in a pane of
  the menu's window, the keys go there.
- **Where it is:** "Open terminal here" in the tab menu (the tab's folder, this pane),
  the pane menu (its active tab's folder) and an app row's menu (the app's folder, the
  focused pane); a Terminals row's "Open" has the places too (moving the shell, the same
  one). Future items ("start a Claude Code session") use the same `places::item`.

Outcome: built. Review of the diff (subagent), all **fixed**: a jitter on a row whose
submenu was just closed with ← or Esc reopened it (only entering a row acts now); a
pending rest reset a submenu the keys had opened and walked; a submenu whose row wasn't
drawn yet could take keys unseen; while resting on another row the highlight and the
keys' target disagreed. Each has a test that fails without its fix.

## Step 5 outcome (2026-11-07)

- **Config:** both workspaces load without errors; discovery finds 35 apps in
  `personal` (3 ms) and 449 in `tadaweb` (0.2 s), 272 of them in `dupes/` (two copies
  of `front-end` and `monitorapp`). Envs are still commented out in `tadaweb.toml`.
- **States:** each state below is covered by a test on real processes or a snapshot.
  Added: wide characters (`terminal.rs`), the run's terminal following its view's
  window, the UI scale and a detach (`tests/runs.rs`), and the no-config screen
  (snapshot `09-no-config-welcome`).
- **Measurements:** [perf/M2.md](../perf/M2.md), "Close". Every threshold passes but
  two: private memory per leak cycle (336 B, falling; GPUI and Wayland, as in M1) and
  R8.2 after `kill -9`: a child that left the run's session survives and is never
  offered at the next start, since only the group leader is recorded
  (`process.rs:281`, `GroupLog::leftovers`). SIGTERM leaves nothing. Open, waiting for
  a decision.
- **Requirements:** R6.1, R6.2, R6.4 and R6.8 done; R1.4 and R1.5 partly (the cluster
  isn't configured yet); R8.2 partly (above).
- **Manual test:** [M2-manual-test.md](M2-manual-test.md).

## States to check

No config; config error; app vs library; missing tool; `.nvmrc` wants a missing Node;
how the env is applied; ANSI colors; input to a prompt; crash toast; stop-caused exit
without toast; spawn failure; port conflict; long and wide output (wraps at pane width);
output in a background workspace, then the view opens; resize on pane drag, UI scale and
detach; vite clearing the screen; wide characters; quit guard with real processes; no
leftovers after quit.

## Review of the draft (subagent, 2026-11-07) and assessment

- "Swap the driver, views don't change" was wrong: the runner relies on fake timers and
  the run view on plain lines. **Accepted**: the run engine and the terminal move to
  step 1.
- The terminal model belongs to the run, not the view (output arrives without a view;
  terminal replies go back to the process). **Accepted**: model in core, element in the
  UI.
- Keys typed into a prompt would trigger run-view shortcuts. **Accepted**: pass-through
  rule (decisions above).
- A terminal grid wraps (R4.4). **Accepted**, user decision: wrap at pane width.
- Render and memory must be measured before building on it (cells are larger than
  today's lines). **Accepted**: terminal trial in step 1.
- Envs are read from the cluster everywhere. **Accepted**: envs move to the workspace.
- A desktop-launched app doesn't see `.bashrc` (nvm, pnpm). **Accepted**: login-shell
  environment (C).
- Ids stored in layouts and pins need a stable scheme. **Accepted**.
- R6.6: "argument" appended a flag; arguments after `task`/`pnpm` mean other things.
  **Accepted**: `{env}` placeholder; argument preferred (user).
- File rewrite needs a gitignore check and a backup. **Accepted for later**: no file
  binding until an app needs one (user).
- R6.4 URLs and unknown override keys. **Accepted** (A).
- Precedence between sources; launch.json entries are not shell lines; Cargo projects.
  **Accepted** (B).
- `Tier::of("preview")` is Hidden. **Accepted**: fixed in step 1.
- Tools depend on the app (Node per `.nvmrc`, pnpm per Node). **Accepted** (C).
- Machine settings out of the shareable workspace file. **Accepted** (A).
- Ports: remember detected ports; structured refusal with "stop the holder".
  **Accepted** (E).
- R8.2: signals to devdeck, the window-close fallback and the quit timeout don't stop
  runs; SIGKILL after 3 s breaks compose. **Accepted** (D, step 3).
- Config reload. **Accepted**: apply on save (user).
- Startup order would break restored tabs; catalog held twice; no config means no
  workspace; state folder collision; ports scanned during render; new run states.
  **Accepted** (steps 1 and 3).
- portable-pty: setsid confirmed, no PR_SET_PDEATHSIG; descendants can escape the group;
  exit before output drained. **Accepted** (D); hard kill is tested in step 5 before
  adding anything for it.
- /proc ports: scan only our pids; root-owned holders; IPv6. **Accepted** (E).
- notify: non-recursive watches. **Accepted** (step 3).
- YAML and JSONC parsers to choose deliberately. **Accepted** (step 1).
- Tests: process, ports and discovery as plain tests with real `sh` children and fixture
  trees, never the user's folders; runner tests keep the fake driver. **Accepted**.
- Interfaces not independent as listed; worktrees sharing one target folder would
  rebuild each other. **Accepted**: step 1 lands every interface, field, `mod` line and
  dependency; each worktree builds `devdeck-core` alone in its own target folder.
