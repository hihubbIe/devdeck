# Control API, phase 1: look

Asked 2026-11-08 (user). Phase 1 of [control-api.md](control-api.md) (R13).

## What

The API exists, knows who calls, and answers **observe** methods. A CLI (`devdeck-ctl`)
and a first MCP server (`devdeck-mcp`) use it, so Claude can look at devdeck. Nothing
changes state through the API yet. Settings shows who is connected and the audit.

The user tests on the real devdeck (the demo has no real shells, so no terminal callers).

## Steps (one commit each)

0. **MCP spike** (thrown away): rmcp's API, a stdio server driven by hand; what Claude
   Code passes to a stdio server (environment), its timeouts, how it treats a server
   that starts while devdeck is down.
1. **`devdeck-api` crate** (no gpui, no tokio): the protocol, the catalog, a client.
2. **Server** in `devdeck-app/src/api/`: socket, identity, policy, dispatch, audit,
   `ApiState`.
3. **Environment**: shells get `DEVDECK_SOCKET` and `DEVDECK_TERMINAL`; app runs don't;
   devdeck removes both from its own environment at start.
4. **Observe methods.**
5. **`devdeck-ctl`.**
6. **`devdeck-mcp`**, and `install.sh` installs both programs.
7. **"API" section in Settings.**

Then the diff review, perf, docs, and the list of commands for the user's test.

## Spike results (step 0)

- rmcp `=3.9.0` (features `server`, `transport-io`; no macros needed): a hand-written
  `ServerHandler` (`get_info`, `list_tools`, `call_tool`) lists a tool built at run time
  (`Tool::new` + `ToolAnnotations{read_only_hint}`) and answers calls with
  `CallToolResult::success/error`; served with `.serve(rmcp::transport::stdio())` on a
  current-thread tokio runtime. Driven by hand over stdio it negotiated `2025-11-25`.
- Claude Code's stdio MCP servers inherit Claude's environment (checked on this
  machine: `/proc/<pid>/environ` of a running server holds the variables of the Claude
  process, minus Claude's own secrets). So `DEVDECK_SOCKET` set in a devdeck shell
  reaches `devdeck-mcp`, and the server is a descendant of that shell: identity works.
- Claude Code docs: tool calls time out after `MCP_TOOL_TIMEOUT` (about 28 h by
  default); a main-conversation call still running after 2 minutes moves to the
  background. Tool lists can change (`list_changed`; interactive sessions refetch).
  Results over 25 000 tokens are cut (`MAX_MCP_OUTPUT_TOKENS`); text over 50 000
  characters goes to a file. Not documented: what Claude Code does with a server whose
  backend is down, so the tool is always listed and answers `unavailable`.
- Claude Code's Bash sandbox may block Unix sockets (`sandbox.network.allowUnixSockets`
  opens one): the CLI from Claude's Bash may need it; MCP isn't sandboxed that way.

## Protocol (frozen by "additive only", so decided now)

- JSON-RPC 2.0, one message per line, at most 1 MiB (read through `take`). No batches
  (`invalid_request`). Messages without an id (notifications) are dropped and audited;
  clients skip incoming messages without an id (events come in P3).
- One request in flight per connection.
- Errors: protocol errors use JSON-RPC's codes; devdeck's use `-32000` with
  `data.kind` (`not_found`, `invalid`, `refused`, `permission_denied`,
  `approval_required`, `approval_denied`, `gated`, `conflict`, `busy`, `expired`,
  `rate_limited`, `unavailable`).
- Every enum a client reads has an `Unknown` fallback; results never refuse unknown
  fields. Params refuse unknown fields (a typo answers `invalid`).
- A method has a base class and the classes it may rise to with its target
  (`escalates_to`), so later methods like `port.stop_holder` fit.
- Views are typed values mirroring `ViewAddr` (`{kind: "run", app, cmd}`…,
  `{kind: "other", ref}`), never the internal string grammar.
- A terminal has a uid from when it's made (`Runner::open_terminal`), kept across shell
  restarts. A caller is a terminal's shell *instance* (terminal uid, shell pid and start
  time); grants (later) bind to the instance.
- `devdeck.info` gives an `instance` id: ids may differ after devdeck restarts.
- Free text is `Untrusted<String>` (schema marker `x-untrusted`): titles (programs set
  them), program names, file and folder names, app, command and script names, note
  names, run arguments, problems. Env values are hidden (names only); run arguments are
  shown, untrusted.
- `api.describe{method?}`: the whole catalog, or one method's schemas.
- `API_LEVEL = 1`.

## Identity

- At accept: `SO_PEERCRED` uid must be ours. The peer is pinned with `SO_PEERPIDFD`
  (Linux 6.5+; a small `getsockopt`); if that fails the caller is Outside (fail closed).
- The connection's thread walks `/proc` up from the peer with no lock: `[(pid, start)]`,
  each parent started no later than its child (a reused pid in the chain breaks it), and
  the pidfd still alive after the peer's own stat is read.
- The UI thread matches the chain against the live shells of every workspace (shell pid
  and start time from the runner): the first match is the caller; none: Outside. No
  shared registry to keep in step.
- On every call the UI thread checks the shell instance still runs; if not, the call
  answers `expired` and the connection closes.

## Policy and scope in P1

- Outside callers: `session.*`, `api.describe`, `devdeck.info` only, until the user can
  raise one with a card (P4). Outside never sees more than a terminal.
- Terminal callers: every observe method, on their own workspace only (others:
  `not_found`).
- A test checks, for every method, that Outside's rights are within a terminal's.

## Socket

- Folder `$XDG_RUNTIME_DIR/devdeck/` made 0700; refused if it exists with another owner,
  another mode, or as a link (`lstat`). No `XDG_RUNTIME_DIR`: the API is off, a toast
  and Settings say why.
- Name: `main.sock` for the default state folder, `demo.sock` for the demo's, else a name
  from the state folder's hash (measure runs, tests). Guarded by its own lock
  (`<name>.lock`, flock): held by another devdeck, the API is off for this one and says
  so; held by us, a stale socket is removed. Socket 0600. The path must fit `sun_path`.
- One accept thread (blocking: idle CPU 0), one thread per connection; at most 32
  connections, of which 8 Outside.
- The server takes its folder as a parameter, and clients take a path: only the
  programs' `main` read the environment. Tests never reach a real devdeck.
- A stop handle (shutdown on the listener) for tests and quit.

## Dispatch and threading

- Transport generic over its sink: tests use a stub, devdeck a channel to one foreground
  task that runs the handler with `&mut App` and answers on a `sync_channel(1)` (a
  dropped sender answers `unavailable`). The UI thread takes a snapshot; the connection
  thread encodes and writes.
- Handlers: a table name → fn, built from the typed methods; a test checks every catalog
  method has one.
- Connections and audit entries reach `ApiState` over the same channel (no polled
  mutex, R1.3).

## Audit

Writer thread, JSON lines in the state folder `api-audit.jsonl`, rotated at 1 MB (one
old file kept). P1 records: connections (rate-limited per caller), refused connections,
permission denials, dropped notifications; later every non-observe call. The last 200
in `ApiState`. The audit list is in time order, newest first: an exception to R4.1.

## Environment

- `pty_driver::run_vars`, shell branch: `DEVDECK_SOCKET` (the socket path) and
  `DEVDECK_TERMINAL` (`<workspace>/<n>`). `PtyDriver` carries both. App runs get neither.
- `scrub_env()`, tested, first thing in `main`, before anything starts a thread.

## Observe methods

`session.hello{client, api_level_min}`, `session.whoami`, `api.describe{method?}`,
`devdeck.info`, `devdeck.problems`, `workspace.list/get`, `app.list/get`,
`run.list/get`, `terminal.list/get`, `ui.get`, `layout.list/get`, `note.list/get`
(names only), `env.list/get`, `editmode.get`, `settings.get`. Lists alphabetical (R4.1).

Git, ports and cluster observe wait for P3 (they need interest handles or scans that
only run while wanted, R1.3).

## CLI

`devdeck-ctl whoami | describe [method] | call <method> [json]`, `--json` for raw
output, `--socket <path>`; finds the socket from `DEVDECK_SOCKET`, else `main.sock`.
Exit codes: 0 done, 1 the API refused (kind printed), 2 no connection. Claude Code's
Bash sandbox may block Unix sockets: use MCP from Claude.

## MCP

- rmcp pinned exactly (it changes weekly), stdio.
- One tool per class the caller may use: P1 `devdeck_observe{method, params}`, method an
  enum of the allowed observe methods, the description listing them in one line each;
  `readOnlyHint`. Short server instructions (what devdeck is, ids, untrusted text).
- Devdeck down at start: the tool is still listed and answers `unavailable`; it
  reconnects when devdeck is back.
- Registration: `claude mcp add devdeck -- ~/.local/bin/devdeck-mcp` (user scope).

## Settings

An "API" section: the socket (or why the API is off), callers connected (alphabetical,
truncated names, R4.4), what each kind of caller may do, the last audit entries.
Keyboard reachable (R5.1), `devdeck-ui` components only (R9.3).

States checked by snapshot: API off, no callers, several callers, a long program name.

## Tests

- api: round-trips, unknown enum values and fields, catalog conformance.
- Identity: the ancestry walk over a fake `/proc` (reused pid, chain broken), shell
  matching, expired instance.
- Transport with a stub sink: folder refused (mode, owner, link), lock held, malformed,
  oversized, unknown method, batch, notification, connection limits, peer pinning
  failure → Outside.
- Dispatcher in GPUI's test context on the demo: every observe method; scope.
- Permission matrix from the catalog; Outside within terminal.
- `run_vars`: shells get the variables, app runs don't; `scrub_env`.
- CLI and MCP against a stub server (`DEVDECK_SOCKET` cleared for their processes).
- Perf: idle CPU and memory with the server listening, before and after.
