# M2 manual test

M2 runs your real apps: the workspaces come from `~/.config/devdeck/workspaces/`
(`tadaweb`: `~/Documents/repos`; `personal`: `~/Documents/private_repos` and
`~/Documents/public_repos`). Runs are real processes in a pseudo-terminal. Layouts, pins,
recent arguments and what ran are saved in `~/.local/state/devdeck`. `devdeck --demo`
still shows the fake data of M1, with its own state.

```sh
cargo build --release && ./target/release/devdeck
```

## Things to try

**Apps** — ctrl+1. The list is a tree of folders.
- → / ← open and close a folder, or click its chevron; a folder shows how many apps it
  holds and whether one runs (dot) or can't (warning).
- Libraries are listed dimmed. `tadaweb` has 449 apps, 272 of them under `dupes/`.
- An app that can't run says why: a missing tool, a Node that `.nvmrc` or
  `engines.node` wants and isn't installed, "vite isn't installed: run pnpm install".
  Node projects have an install command.
- Right-click an app: Open, Pin, "Open terminal here".

**App view** (⏎ on an app)
- ↑↓ select, ⏎ runs, s stops, r restarts, o opens the output (the terminal icon),
  h shows hidden commands.
- Cargo projects with several binaries get one run per binary.

**Run tab**
- Real colors, progress bars, prompts: i (or a click in the output) types into the
  program, ctrl+shift+escape leaves it.
- Arguments: a or the `args…` field. Recent ones, none, or type new ones; choosing
  restarts the run with them. npm and `cargo run` get them after `--`.
- A run that ends with 0 shows a check, "succeeded in 14 s" and a toast; a failure the
  error icon and its code, and a toast that leads to it, even from the other workspace.
- Hold shift over the output: links are underlined; shift+click opens one.
- Select with the mouse; ctrl+shift+c copies (ctrl+c too while something is selected).
- The output wraps at the pane width: resize the pane, change the UI scale (ctrl+= /
  ctrl+-), detach the tab; the program gets the new size.

**Terminals**
- ctrl+shift+t opens your shell in the focused tab's app folder.
- "Open terminal here" in a tab's, a pane's or an app's menu: a click opens it right
  there; rest on it (or press →) for the other places: every pane, a new window.
- Closing a terminal's tab, or the detached window holding it, ends the shell.
  Switching layouts doesn't.
- ctrl+5 lists every shell of the workspace, shown or not: its folder, what runs in it,
  where it shows. Open (with the same places) or End.

**Tabs and windows**
- Drag a tab along its strip to reorder, like a browser.
- The window has a 1 px border when it isn't maximized.

**Ports** — ctrl+3. Ports your runs listen on. Starting a command whose port is held
says who holds it and offers to stop it when it's one of yours.

**Config** — edit a workspace file while devdeck runs: it applies on save. A new
project folder appears without a restart. A broken file is one toast and the Problems
view.

**Quit** — ctrl+q or the close icon. With something running (a run, or a shell running
a program) it asks "Stop all and quit" or "Cancel"; idle shells just end.
- After a crash or `kill -9` of devdeck, the next start offers to kill what it left,
  one row per run, daemons included.
- A run that ends on its own (a build) keeps the daemon it started (sccache, Gradle,
  Nx) until devdeck quits; stopping a run ends everything it started. What you detach
  on purpose from a terminal tab (tmux, `setsid`) is left alone.

## Known limits of M2

- Envs are commented out in `tadaweb.toml`: no env button until you set them. How an
  app gets its env is per app (`[apps.<id>] env = …`), none configured yet.
- The cluster is still M1's fake data (`--demo`); real clusters are a later milestone.
- The stress cycle used for the leak test doesn't cover terminal tabs and submenus.
- Measurements: [docs/perf/M2.md](../perf/M2.md).
