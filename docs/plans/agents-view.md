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

## 6b (2026-11-11, the user: "fix the bugs, then make cool, useful stuff, data viz")

- **Fix:** quitting asks while a session works or waits on you (the quit guard counted
  an agent's terminal as an idle shell: its program is the terminal's own process).
- **Reports:** `devdeck-ctl report <event>` (from the agent's hooks and status line,
  its JSON on stdin) calls `agent.report` (API level 8, class Present). Only from inside
  one of devdeck's terminals; it says nothing, always succeeds (a failing hook can block
  the agent), and a status line gets back a line (model · context · cost). Events:
  started, working, needs_you, your_turn, ended, status. What the JSON says is the
  agent's config: `[agents.<id>.fields]` (session, title, model, context, cost, added,
  removed, message, prompt, asks, pr: JSON pointers) and `[agents.<id>.limits.<name>]`
  (used, resets). Reported states win over the terminal's signs: a working session is
  never stopped, however quiet. A `started` with another id (`/clear`, `/resume`
  inside) makes the session resume that conversation.
- **Facts** kept with the record; **history**: seconds worked per minute, the last hour.
- **Overview** (`agents`, the Agents view with nothing selected, "overview" in its
  header; also a tab): Now (counts by state, cost, the last hour of all sessions as
  activity bars), Waiting on you (what each asks), Usage (each limit, a bar with a tick
  at 80 %, when it resets), Sessions (state, name, agent and folder, context bar, cost,
  +/− lines, the last hour, last active; a row goes there), Latest answers.
- **Attention:** a count of sessions waiting on you over the Agents icon (drawn over
  it, R9.10); a toast with "Go there" when a session off screen needs you or is done.
- New component: `devdeck_ui::activity_bars`.
- Claude's side, all config: `~/.config/devdeck/claude-agent.json` (passed with
  `--settings`: the bell, hooks and a status line calling `devdeck-ctl report`; hooks
  from several settings add up, per Claude's hooks docs).

## Out of scope (later)

A split of sessions inside the Agents view, starting a session with a task from where
the problem is (a failed run, logs, the git panel), sessions per app in the app view,
a worktree per session, sessions made outside devdeck, OS notifications.

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
