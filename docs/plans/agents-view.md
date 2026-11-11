# Agents 6a: agent sessions, a full view of their own

Goal: run AI agents (Claude Code, or any agent the machine configures) as sessions devdeck
knows: started in a folder, listed in an Agents full view, shown there or in a pulse,
resumed when shown again. devdeck knows nothing of Claude: what an agent is, how it
starts and resumes, is `machine.toml`.

Requirements: R4.1, R4.2 (the list), R4.10 (full view), R5.1 (actions), R6.13 (places),
R1.5 (config), R9.10, and a new R6.14 (below).

## Decisions (2026-11-11, the user: "think and figure what is the right way")

- **Agents is a full view** (R4.10), as Terminals is: the list, then the selected session
  itself. It is not a pulse: nothing of it is saved as one. A session also opens in a
  pulse ("Open in pulse", or "<agent> here" from a pane's menu).
- **A session is a record, its process comes and goes.** The record (its id, agent,
  folder, title, terminal number, created, last active) lives in the workspace's state
  (`workspaces/<ws>/agents.json`). Its process runs while a view shows it. Off screen, it
  keeps running while it works or waits on the user (a bell since the last key), and
  stops once it has been quiet for the agent's `idle_stop` (60 s by default). Shown again,
  it starts with the agent's `resume` arguments. This is Claude Code's agent view's rule
  (its supervisor stops an unattached idle session after about an hour), made short
  because the user wants no agent running without a view.
- **One process per session.** A session's terminal is one of the runner's terminals,
  numbered like shells, running the agent's line instead of a shell: two views of a
  session share it, and the control API knows a caller inside it as that terminal
  (an MCP server inside the agent keeps devdeck's terminal rights).
- **State without the agent's help.** Output in the last 1.5 s: working. A bell (the
  agent's own notification, e.g. Claude's `preferredNotifChannel = "terminal_bell"` set
  through its `--settings`): needs you, until a key. Quiet after working while off screen:
  done, until shown. The title is the user's rename, else the terminal title the agent
  sets (Claude sets it to its session name), else the agent and the folder.
- **The list**: Pinned + All, alphabetical by title (R4.1), a dot by state.
- **Menus**: wherever "Open terminal here" is, a separator then "<agent> here" for each
  configured agent, with the same places submenu.

Config (machine.toml):

```toml
[agents.claude]
name = "Claude Code"
command = "claude"
args = ["--settings", "{\"preferredNotifChannel\": \"terminal_bell\"}"]
start = ["--session-id", "{id}"]   # {id}: the session's id, a UUID devdeck makes
resume = ["--resume", "{id}"]
idle_stop = 60                     # seconds off screen and quiet before it stops
```

## Out of scope (6b, later)

Reports from the agent through the control API (`agent.report` from its hooks: exact
states, title, last message), a split of sessions inside the Agents view, sessions made
outside devdeck, OS notifications.

## Files

Core: `agents.rs` (new: kinds, lines, records, state), `config.rs` (`[agents]`),
`state.rs` (agents file), `terminal.rs` (bell, title), `view_addr.rs` (`agent/<id>`).
App: `agents.rs` (new: the workspace's sessions), `runner.rs` (a terminal running a
program), `services.rs`, `deck.rs`, `views/mod.rs`, `sidebar.rs`, `window_view.rs`,
`environment.rs`. UI: the `sparkles` icon (Lucide; copied from the Lucide 1.43.0 set in
`gpui-kit-assets`, the same drawing).

No OS-specific code (R11.6): the runner's PTY and process stop already go through
`devdeck-sys`.

## Tests

Core: config parsing and errors, the line's quoting and `{id}`, the state rules (working,
needs you, done, stop due), records' order and titles, bell and title from a terminal.
App: "<agent> here" makes a session started with `start`, shown in Agents; hidden and
quiet past `idle_stop` it stops; shown again it starts with `resume`; a working or
needing session isn't stopped; Terminals doesn't list it; rename and delete.

Snapshot: none (the user checks it by hand).
