# M1 manual test

Everything runs on fake data: no process is started and nothing touches a cluster. The
app saves its layouts, pins and settings in `~/.local/state/devdeck` and
`~/.config/devdeck` (remove them to start fresh).

```sh
cargo build --release && ./target/release/devdeck
```

## Things to try

**Window and panes** (M0, still there)
- Drag tabs between panes and onto pane edges; drag the ⠿ grip to move a pane.
- Collapse (▴), maximize (□), detach (⇱) and re-attach (⇲ in the detached window).
- alt+arrows move focus, alt+shift+arrows move the tab, ctrl+\ splits.

**Settings** — ⚙ in the title bar or ctrl+,. UI scale steps (ctrl+= / ctrl+- / ctrl+0), theme.

**Workspaces** — `work ▾` in the title bar or ctrl+alt+1/2.
- `personal` has no cluster: no Cluster activity, no env button.
- A detached window of `work` closes while `personal` shows and comes back after.

**Sidebar** — ctrl+1 Apps, ctrl+2 Cluster, ctrl+3 Ports, ctrl+4 Layouts, ctrl+b hides it.
- ↑↓ select, ⏎ opens, p pins, / filters, r shows running apps only. Esc clears the filter, a second Esc goes back to the panes.
- Right-click a row for its menu. Drag a row onto the other section to pin or unpin, or drop it on a pane to open it there.
- `remotedesktopapp` is disabled: "docker not found".

**Env** — `@dev35 ▾` in the title bar (ctrl+e). hades, dls and lab are protected (⊘). dev36 is unreachable and lab forbids metrics, on purpose.

**App view** (⏎ on an app)
- ↑↓ select, ⏎ runs, s stops, r restarts, o opens the output, h shows hidden commands.
- `boardservice · run` crashes after about 4 s with a toast; "View output" opens it, even from the other workspace.
- `pagerendererservice · test` fails with exit 1.

**Service view** (⏎ on a deployment)
- t edits the tag, ▾ suggests tags, ctrl+s shows the exact kubectl command, ⏎ deploys.
- A tag containing `broken` fails with ImagePullBackOff; u undoes.
- On hades, writes ask you to type `hades` first.
- l opens logs: 1–4 level filter, / find, n / shift+n jump between matches, p pauses.

**Matrix** — the "matrix" button in the Cluster sidebar. Click a tag to highlight it everywhere, double-click a cell to open the service.

**Actions**
- ctrl+k palette: try `entity`, Tab on an app to list its commands, `event run @33`, `layout`, `split right`.
- ctrl+/ or ? opens the cheat sheet. The bottom bar shows the keys that apply where focus is.
- Right-click anything: tabs, tab strips, rows, commands, pods, matrix cells, activity icons. shift+F10 opens the menu of what has focus.
- ctrl+shift+s sends a tab, ctrl+alt+shift+s a whole pane. Pick by number, by the list, or by clicking a pane in any window.

**Layouts** — `default ▾` in the title bar, or ctrl+4.
- Save layout as…, switch, rename, delete.
- "Keep in every layout" is in a tab's menu.

**Quit** — ctrl+q or ×. Things are running (the fake servers), so it asks "Stop all and quit" or "Cancel".

## Known limits of M1

- All data is fake. Shell, k9s, port-forward, replicas and restart are disabled and say why.
- A tab closed by a layout switch loses its unsaved edits: tag, find text, unlock.
- Pins of a renamed layout don't follow the rename.
- Some glyphs (✖ ◐ ⇲ ⇱ ⠿) come from a system fallback font, not the bundled one.
