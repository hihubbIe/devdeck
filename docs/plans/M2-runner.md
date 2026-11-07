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
