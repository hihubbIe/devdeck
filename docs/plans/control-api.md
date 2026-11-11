# Control API (Claude control)

Asked 2026-11-08 (user). Comes after Files and notes. Replaces the roadmap's "Claude
control (MCP)" entry: MCP becomes one client of a generic API.

Status: researched, reviewed, decided (user, 2026-11-08: the six proposals below). Next:
P1. Requirements: R13 in docs/REQUIREMENTS.md; design: "Control API" in docs/DESIGN.md.

## In short

- devdeck gets a **generic control API**: every thing it shows and every thing it does,
  addressed by ids, on a local socket. It knows nothing about agents.
- **Clients are separate programs** that only know the API: a CLI (`devdeck-ctl`) and an
  MCP server (`devdeck-mcp`) for Claude Code. Dropping MCP one day touches nothing else.
- **Who is calling** is known from the process tree, not from a password: a program
  started from devdeck's terminal 3 *is* terminal 3. Nothing to steal.
- **What a caller may do** depends on the kind of operation (8 classes, from "look" to
  "destructive"), on who started the thing it acts on, and on devdeck's own rules (edit
  mode, protected envs). Risky calls show a card in devdeck that only the user can answer.
- **Built in 7 phases**; Claude can use it from phase 1 (read only).

## Decisions (user, 2026-11-08)

All six taken as proposed.

| # | Question | Decided |
|---|---|---|
| 1 | How is a caller identified? | By process tree, no tokens ([Identity](#identity)) |
| 2 | Default permissions of an agent in a devdeck terminal | The table in [Defaults](#defaults) |
| 3 | Programs not started from devdeck (CLI in another terminal app) | Look only; a card in devdeck can raise one program, for its lifetime |
| 4 | May an agent open a terminal running a command of its choice, and type into terminals it opened, without asking? | Yes: it can already do it in its own shell; this way the user sees it |
| 5 | Which files can be read through the API? | The workspace's folders only |
| 6 | Copy Claude Code's IDE protocol (open file, selection sent to Claude)? | Later, not in this plan (unofficial protocol) |

## Contents

1. [Principles](#principles)
2. [What it protects against](#what-it-protects-against)
3. [Pieces](#pieces)
4. [Socket and messages](#socket-and-messages)
5. [Identity](#identity)
6. [Permissions](#permissions)
7. [Ids](#ids)
8. [Methods](#methods)
9. [Changes the app needs first](#changes-the-app-needs-first)
10. [MCP client](#mcp-client)
11. [Phases](#phases)
12. [Tests](#tests)
13. [Unsure](#unsure)
14. [Sources](#sources)

---

## Principles

1. **The API is devdeck's model, not its screen.** It names things by id ("run
   `front-end/graph` `npm:dev`", "pane 4 of layout X"), never "the focused pane". Every
   operation becomes a function both the UI and the API call; the UI only works out
   *which* ids the user means. Today many operations live in views and modals: they move
   out (the app becomes API-first).
2. **Generic.** Nothing in the API is for agents. Any client can use all of it.
3. **devdeck checks every call itself.** What a client says about itself is never
   trusted.
4. **No "run any action by its id" method.** It would let a client press "confirm" in a
   modal. Every operation is its own method with typed arguments.
5. **Grows without breaking.** An integer `api_level`; methods and fields are only ever
   added (Neovim's rule); new ones can be marked experimental. The one exception (user,
   2026-11-11, R13.2): a feature devdeck drops takes its methods, fields and topics with
   it, at a new level, listed in `API_LEVEL`'s comment (level 5: edit mode's
   `editmode.get` and `edit_mode` topic).

## What it protects against

The permissions are **not a sandbox**. Claude in a devdeck terminal already has the
user's full power through its own shell tool (Claude Code's permissions govern that).
Any program of the user can read devdeck's files, and on X11 can even click its buttons.

They protect against:

- an agent, or an agent tricked by text it read (a log, a note, a branch name), doing
  something unexpected or destructive **through devdeck**: deploy, type into the user's
  other shells, stop the user's dev server, delete notes;
- any client getting more than it needs;
- things happening unseen: every call that changes something is in an audit log.

devdeck holds private data, shows untrusted text (logs, output, notes) and can act on the
outside: the three ingredients of prompt injection. So **what a client reads never
changes what it may do**.

## Pieces

| Piece | Holds | Depends on |
|---|---|---|
| `devdeck-api` (crate) | The protocol: request, result and event types, the method catalog (name, argument and result schemas, class, level), error kinds, id types, a small client | nothing of devdeck |
| `devdeck-app/src/api/` | Socket server, identity, permissions, approvals, one dispatcher per domain, events, audit | the app (the only part that does) |
| `devdeck-ctl` | The CLI | `devdeck-api` only |
| `devdeck-mcp` | The MCP server for Claude Code (rmcp 3.x, stdio) | `devdeck-api` only |

Threading. Everything in devdeck lives on the UI thread. The socket's thread passes each
request to it through a channel (as the file watcher does). The UI thread only takes
what it needs (a snapshot); the connection's thread turns it into JSON and writes it; the
audit log has its own writer thread. Model observers exist only while some client
listens, timers only while something waits: idle CPU stays 0 (R1.3).

## Socket and messages

- **Socket**: `$XDG_RUNTIME_DIR/devdeck/<instance>.sock`, folder 0700, socket 0600,
  never an abstract socket. The caller's user must be ours (`SO_PEERCRED`).
- **No HTTP.** A local web port can be reached from web pages (DNS rebinding, CSRF);
  real MCP tools had such holes.
- **Messages**: JSON-RPC 2.0, one message per line (MCP's own format on stdio).
- **`api.describe`** returns the whole catalog: every method, its schemas, its class, and
  whether this caller may use it.
- **Errors**: protocol errors use JSON-RPC codes. devdeck's errors carry a `kind`:
  `not_found`, `invalid`, `refused` (with the reason the UI would show),
  `permission_denied`, `approval_required`, `approval_denied`, `gated` (a protected env,
  main/master), `conflict`, `busy`, `expired`, `rate_limited`,
  `unavailable` (not built yet).
- **Reading text is always bounded**:
  - `limit` and `tail`, and a cursor to read only what came since the last read. The
    cursor knows the terminal's size and run: after a resize or a restart it answers
    `expired` instead of returning shifted rows.
  - Plain text (colour codes removed) by default; a `screen` mode returns what a
    full-screen program (htop, k9s) shows.
  - Search by regex with lines around each match.
  - A `truncated` flag; wrapped rows joined back into lines.
- **Untrusted text is marked**: every free-text field (output, logs, notes, branch names,
  script names, tags) is flagged untrusted in the schema. Secrets devdeck knows of are
  hidden.
- **Waiting is one mechanism, jobs**: long operations, approvals and "wait until ready"
  all return a job (`job.get`, `job.wait`, `job.cancel`). A wait
  that times out returns the state and the output so far; the process keeps running.
- **Events**: `events.subscribe{topics}`. Notifications are numbered; each client has a
  bounded queue; when it overflows the client gets `lagged` and reads the state again.
  Output is sent in batches about every 100 ms, only while there is some.

## Identity

Passwords (tokens) in the environment can be stolen: from another shell's
`/proc/<pid>/environ`, from a file with `cat`, or from a terminal where someone ran `env`.
So devdeck doesn't use them.

**The caller is found from the process tree.** The kernel gives the caller's process
(`SO_PEERCRED`, held as a pidfd so a reused pid can't fool it). devdeck walks up its
parents to a shell it started: the caller is that terminal, `terminal(workspace, n,
uid)`. Claude Code's MCP server is a child of Claude, which runs in that shell, so it is
recognised.

- **Outside**: anything that doesn't descend from a devdeck terminal (the CLI in another
  terminal app, a program that detached itself, tmux sessions). It may only look, unless
  the user raises it with a card in devdeck, for that one program while it runs. Escaping
  the tree never gives more rights: outside is never above a terminal's default.
- **Environment**: shells get `DEVDECK_SOCKET` and `DEVDECK_TERMINAL`, only to find the
  socket; they grant nothing. App runs (dev servers, npm scripts) get neither. devdeck
  removes them from its own environment at start (a devdeck started inside a devdeck
  terminal would otherwise pass them on to its runs).
- **Grants end with the shell.**
- **Limit**: the user and the agent in the same terminal are the same caller.

## Permissions

### Classes

Every call has one class, worked out when it arrives from the method **and** what it
acts on (stopping a port held by a devdeck run is `run`; stopping a foreign process is
`destructive`).

| Class | What | Examples |
|---|---|---|
| observe | State, no content | apps, runs and their state, layout, ports, git status |
| read | Content | output, logs, notes, files, selection |
| present | Only what the user sees | open/close/focus a tab, split, switch layout, sidebar, scale. Never moves keyboard focus while the user types; opens in the background by default |
| organize | devdeck's own data | notes, layouts, pins, groups |
| run | Configured commands and shells | start, stop, restart, open a shell |
| type | Arbitrary execution | text or keys into a terminal or process; run arguments set by the API; a terminal opened with a command |
| remote | Git writes | fetch, pull, switch, new branch, push |
| destructive | Can't be undone, or shared | delete a note or layout, stop a foreign process, force push, git abort, any cluster write, exec in a pod, quit |

### Who started it

Each run and terminal remembers who started it (the user, or which terminal). Acting on
what the caller started itself follows its class; acting on what the user or another
caller started asks. Stopping a shell with a program running in it asks, as quitting
does (R6.12).

### Checks, in order (all must pass)

1. The caller's policy for the class (allow, ask, deny).
2. Scope: the caller's own workspace (R2.1); protected envs excluded by default.
3. devdeck's own rules, exactly as for the UI, in the model: a dangerous action confirmed
   (R5.6: the user's approval is its confirmation), protected env, main/master never
   pushed.
4. Approval, if the policy says ask.
5. Rate limits.
6. Audit log (time, caller, process, method, arguments, decision, result), shown in the
   app.

### Never through the API

Unlocking a protected env (unlocks are per caller: the API can't
use the user's), changing permissions, reading or clearing the audit log, answering
approvals, and the UI's confirm, modal and menu actions.

### Approvals

- **One `Approvals` service** for every confirmation; today's modals become one way to
  show it.
- **The model functions that destroy require a proof** (`Approval<SetImage>`, holding its
  own arguments) that only the user's answer in the UI can make. No path skips it, the
  UI's included.
- **An approval is used once**, expires in seconds, and is bound to: the caller, the
  method, the exact arguments, and the state it was decided on (push plan, deployed
  tag, process holding the port). If that state changed when it runs: `conflict`.
- **The card**:
  - drawn in its own area, which `ui.notify` can never draw in;
  - says which caller asks;
  - shows the arguments in full, wrapped, control and right-to-left characters shown
    escaped;
  - never takes focus; Allow is never Enter or `y`; reachable by keyboard (R5.1);
  - answers: **once**, **for this terminal for N minutes** (not offered for typing into
    someone else's terminal, nor for protected envs), **deny**.
- **No asking for more rights through the API**; permissions change only in the API
  access panel. Pending cards per caller are capped, with a pause after denials.

### Defaults

For an agent in a devdeck terminal (decision 2):

| Class | Default |
|---|---|
| observe, read, present | allow |
| organize | allow on its own data; ask before writing over a note the user is editing |
| run | allow on what it started; ask on the user's |
| type | allow into terminals it opened; ask once per terminal otherwise; refused into its own shell (it could answer its own permission prompts) |
| remote | ask |
| destructive | ask: the user's approval is the action's confirmation (R5.6) |

Setting the current env to a protected one, or starting a run in one, always asks; a toast
says who did it.

Levels are presets of this table: **observer** (observe), **viewer** (+ read, present),
**operator** (+ organize, run), **full** (all).

## Ids

All per workspace except ports and files.

| Thing | Id |
|---|---|
| Workspace | its config id (switching is by position today) |
| App, command | app id, command id |
| Run | (app, command) |
| Terminal | (n, uid): a number reused later is not mistaken for the old one |
| View | the existing view addresses (`run/…`, `file/…`, `note/…`) |
| Pane, window | (layout id, pane id): saved layouts need no change |
| Layout, note | their ids (UUIDs) |
| Repository | its root, only roots of known apps |
| Cluster | env; (env, deployment); (env, pod) |
| Port | its number |
| Job | job id |

`session.whoami` tells a caller which terminal it is, in which workspace, and where it
shows.

## Methods

What a client has to be able to do, as examples: start the API and the front, wait until
they answer, give the URL · why did the build fail · show this file at line 120 · add
this to mf-entities-graph's notes · logs on the left, server on the right · is staging
running my branch · deploy tag X to staging · ask me which env · what's on port 3000.

Methods are named `domain.verb`. The class is in brackets when not obvious.

### Session and devdeck

- `session.hello`, `session.whoami`, `api.describe`
- `devdeck.info` (version, instance), `devdeck.problems`
- `devdeck.quit` [destructive]

### Workspaces and apps

- `workspace.list`, `workspace.get` (folders, envs, config)
- `workspace.activate`, `workspace.rescan` [present]
- `app.list`, `app.get` (commands, URLs, kind, why unusable, notes, repository)
- `app.open_url` [present]

### Runs

- `run.list`, `run.get` (state, process, ports, last exit, duration, command line with env
  values hidden)
- `run.output`, `run.search` [read]
- `run.start{env, args}` [run]; arguments are a list, quoted for the shell; set by the
  API they make it [type] and are not saved to recent arguments unless approved
- `run.stop`, `run.restart` [run]
- `run.wait{until: exit | output matches | port listening, timeout}`
- `run.input` [type], `run.paste` [type] (the Share with rules: pasted as one block,
  control characters removed, no Enter)

### Terminals

- `terminal.list`, `terminal.get` (folder, program running, who started it)
- `terminal.output`, `terminal.screen`, `terminal.search` [read]
- `terminal.open{dir, place, command?}` [run; with a command, type]
- `terminal.input`, `terminal.paste` [type], `terminal.stop` [run]
- `terminal.wait{until: idle | output matches, timeout}` (idle: the shell is back at its
  prompt)

### What's on screen

- `ui.get`: windows, pane tree, tabs (view, title, shown), focused pane
- `ui.selection` [read], `ui.toasts`
- `view.open{target, place, activate: false}` [present]: the target is typed (app, run,
  file, note…) and must be something devdeck knows; terminals open only through
  `terminal.open`, repositories only if they're an app's
- `view.close`, `view.focus`
- `pane.split`, `pane.close`, `pane.collapse`, `pane.maximize`, `pane.resize`,
  `pane.move_tab`; `window.detach`, `window.attach`, `window.close`
- `sidebar.show`; `file.toggle_rendered`; logs view state (pause, follow, level, find);
  matrix highlight; `settings.set{scale, theme}`
- `ui.notify{text, kind}`: labelled with the caller
- No `user.ask`: a client asks the user in its own session (dropped by the user,
  2026-11-09)

### Layouts and sidebar

- `layout.list`, `layout.get`; `layout.switch` [present]
- `layout.save_as`, `layout.rename`, `layout.keep` [organize]; `layout.delete`
  [destructive]
- `sidebar.pin`, `group.create`, `group.rename`, `group.move`, `group.delete` [organize]
  (deleting a group keeps its contents)

### Files and notes

- `file.open{path, line, range}` [present] (going to a line is new)
- `file.read` [read]: workspace folders only, path resolved first so a link can't lead
  outside
- `note.list`, `note.get`, `note.read`
- `note.create`, `note.write{base_revision}`, `note.append`, `note.rename` [organize];
  `note.delete` [destructive]

### Git

- `git.repos`, `git.status`, `git.branches`
- `git.fetch`, `git.pull`, `git.switch`, `git.create_branch` [remote]
- `git.push` [remote; destructive when the plan is to replace the remote branch]
- `git.abort` [destructive, except a bisect]

### Ports and leftovers

- `port.list`, `port.get`; `port.stop_holder` (class from who holds it)
- `leftovers.list`; `leftovers.stop` [destructive]

### Envs and cluster

Writes and real logs answer `unavailable` until M3 steps 4–5 exist.

- `env.list`, `env.get`; `env.set_current` [run; asks for a protected env]
- `deployment.list`, `deployment.get`, `pod.list`, `pod.get`, `pod.metrics`,
  `rollout.status`, `deploy.history`, `matrix.get`
- `logs.read{pods, since, level, grep, limit}` [read]
- `deploy.set_image`, `deploy.undo`, `deploy.restart`, `deploy.scale`, `pod.exec`
  [destructive]
- `port_forward.start`, `port_forward.stop` [run; asks on a protected env]
- `k9s.open` [run; read only]

### Settings, jobs, events

- `settings.get` (`editmode.get` removed at level 5, with edit mode)
- `job.get`, `job.wait`, `job.cancel`, `job.list`
- `events.subscribe`, `events.unsubscribe`

### Event topics

Workspace, run state and exit, run output, terminal, layout, tab shown or hidden, ports,
git, deployments, pods, rollout, notes, settings, toasts, jobs.

## Changes the app needs first

The "operations layer": what the UI does, done by functions that take ids.

- Starting a shell moves from the run view into `Runner`.
- The logs text moves out of the logs view into a model (M3 step 4 needs it anyway).
- The protected-env unlock moves out of the service view into a grant per caller.
- Git writes and deploys take their confirmation from `Approvals`.
- Share with works without a window.
- New: note revisions, opening a file at a line, the selection readable, toast history
  with ids, typed events (most models only ask for a redraw today), who started each run
  and terminal, interest handles held by subscriptions and one-shot reads.
- Ids: workspace by id, a uid per terminal.
- Run arguments from the API as a list quoted for the shell (the UI keeps "typed as is").

Found on the way, exists today apart from the API: git reads obey a repository's own
`core.fsmonitor` setting, which can run a command. devdeck already ignores the user's and
the system's git config, not the repository's. Adding `-c core.fsmonitor=false` closes it.

## MCP client

- **Tools**: a few chosen ones for what agents do most (overview, runs with start, stop
  and wait, output read and search, open, layout, notes, git, cluster),
  plus **one generic tool per class** (`devdeck_observe`, `devdeck_run`,
  `devdeck_destructive`…) that reaches every method of that class. A new API method is
  usable at once, and Claude Code's "always allow" stays limited to one class.
- The list of chosen tools lives in `devdeck-mcp`; the API doesn't know about it.
- Tool hints (read only, destructive) come from the class. Only the tools this caller
  may use are listed.
- A call that needs approval answers right away with a job; the agent then waits with
  another call (no call held open for minutes).
- Later: MCP resources for reads; Claude Code's IDE protocol (decision 6); events pushed
  to Claude (Claude Code "channels").

## Phases

Each ends with the user's manual test (WORKFLOW).

| Phase | Contents |
|---|---|
| P0 | Decisions; R13 in REQUIREMENTS; DESIGN section; the catalog written out |
| P1 | `devdeck-api`, socket server, identity by process tree, audit log, environment cleanup, observe methods, CLI, a first MCP with the per-class tools, API access panel (callers and audit, read only). **Claude can use it here** |
| P2 | Bounded reads with untrusted marks and hidden secrets: output, screen, search, notes, files |
| P3 | Operations layer, who-started-what, events, jobs, waits; present, run and organize methods |
| P4 | `Approvals` and its proofs, the approval card; type, remote and destructive methods: git, ports, deletes, leftovers |
| P5 | Chosen MCP tools, server instructions, setup docs |
| P6 | Cluster methods, as M3 steps 4–5 land |

## Tests

- Catalog: every method has schemas, a class and a handler.
- Permissions: a matrix generated from the catalog (method × caller × who started it ×
  devdeck's rules).
- Approvals: changed state, reuse, a proof used for another call.
- No confirm, modal or menu action reachable.
- Every call that changes something is audited.
- Socket: wrong user, broken or oversized messages, a client that reads too slowly.
- Dispatchers in GPUI's test context; the MCP client against a fake server.

## Unsure

- Which MCP version Claude Code speaks today (rmcp speaks both the old and the
  2026-07-28 one).
- Whether rmcp's `#[tool]` takes the read-only and destructive hints, or they're set by
  hand.
- Whether `SO_PEERPIDFD` works for a caller that has already exited.
- Claude Code's time limit on an MCP call (hence approvals answered at once with a job).

## Sources

- devdeck code and docs: an inventory of everything the app shows and does, where each
  operation lives, and its gaps (focus-only actions, state held by views, ids per layout,
  confirmations only in modals).
- Local control APIs: Neovim (`nvim_get_api_info`, its compatibility rule), kitty remote
  control (per-password allowed actions), WezTerm `cli` (`WEZTERM_PANE`), tmux control
  mode (paused panes), i3 IPC, Chrome DevTools Protocol (Chrome 136 closing its debug
  port to stop cookie theft), Docker's socket.
- Agent tools: JetBrains MCP server (run configurations with wait and timeout, a router
  tool), VS Code agent tools (128-tool limit), Zed and ACP (terminal create, output,
  wait, kill, with byte limits), the Claude Code IDE protocol (reverse-engineered),
  iterm-mcp, tmux-mcp, terminalcp, mcproc (wait for a log line), the Kubernetes MCP
  servers (read-only and non-destructive modes, confirmation rules).
- MCP 2026-07-28 (tool hints, structured output, elicitation, tasks, stdio), its security
  advice (no localhost HTTP, least privilege), Simon Willison's "lethal trifecta".
- An adversarial review of the first draft, whose points are all in this version.
