# Control API, phase 3: act

Drafted 2026-11-09. Phase 3 of [control-api.md](control-api.md) (R13.1, R13.5–R13.8,
R13.10, R13.11). API level 4 for the whole phase.

## Where we are (paused 2026-11-09)

- Done: steps 1–7 (`cfe023d`, `02be73b`, `f50cf9b`, `aad09af`, `6d26748`, `5a49513`, step
  7's commit); each has an "as built" section at the end of this file.
- Next: step 8 (methods), then 9 (clients), then the phase's close (diff review, perf,
  docs, the user's manual test).
- Open points carried forward:
  - Linux: GPUI ignores `WindowOptions::focus`, so a window the API opens may take the
    keyboard; step 8 decides whether `window.detach` and a `layout.switch` that opens
    windows are refused there.
  - A workspace switch by the API keeping the user's focus needs its own rule (step 8,
    `workspace.activate`).
  - The notes' folder is watched only once it exists (made with the first note).
  - `docs/PLATFORMS.md` should list the hang-up check (`recv` with `MSG_PEEK`) in
    `api/transport.rs`.
  - Steps 2–5 were reviewed by the lead, not a subagent (Claude Code's permission
    classifier refused it): an independent review before the phase closes.
  - No test yet for keys held in a detached window or an inactive workspace.
  - Deleting a layout has no confirmation in the UI; whether to add one is the user's.
- To resume: "continue the control API phase 3 from `docs/plans/control-api-p3.md`".

## What

Claude can change what devdeck shows, open and stop its own terminals, set the current
env, and organize notes, layouts, pins and groups; it can wait until a run is ready or a
terminal is idle, and follow events. Every operation is one function taking ids, needing
no window or focus: the UI and the API call the same one (R13.1).

No `Approvals` service in this phase: it comes in P4 with the card (decided by the user,
2026-11-09). A call that would ask answers `approval_required`, as today: stopping the
user's terminal, setting a protected env, writing a note the user is editing. None of
them needs the service.

## Where things are today (inventory, 2026-11-09)

- Panes, tabs, windows: pure functions on `Layout` (`devdeck-core/src/layout/mod.rs`),
  applied through `Deck::apply` (`deck.rs`), which needs no window. No `split`: a split is
  `move_tab(p, i, Drop::Edge(..))` (`window_view.rs` `split_active`). The pane actions
  are `WindowView` listeners on the focused pane; `focus_dir`/`move_tab_dir` need the
  window's geometry.
- `Layout::open` and `activate` always focus the pane, and `WindowView::sync_view_focus`
  gives the keys to a newly active view while focus is in the pane area: an API open
  would take the keys from the user's terminal.
- Layouts: `LayoutBook` is pure; `Deck::save_layout_as`, `switch_layout`,
  `rename_layout`, `delete_layout`, `toggle_keep` work from `&mut App`. Save-as and
  rename go through name prompts (window). Deleting a layout has no confirmation.
- Pins and groups live on the `Sidebar` view entity and act on the activity shown
  (`self.activity().list_id()`).
- Notes: `Notes` owns the text; `set_text` saves after 500 ms. No revision, no append,
  not watched on disk. `NoteView`'s source editor copies the text once: a model write
  doesn't reach it, and the next keystroke overwrites it.
- Files open only by drop (`drops.rs` `open_files`); no going to a line.
- Terminals: `Runner::open_terminal` only records one; the shell is started by its view
  (`RunView::start_if_restoring`), which also gives its size. Stopping: `Runner::stop`,
  no confirmation. Terminals have a uid, no `Starter`.
- Confirmations: modals per window (`devdeck-ui` overlay `push_modal`): quit guard,
  git pull/push/switch/abort, deploy, protected-env unlock (typing the name), note
  delete, port holder, leftovers, edit mode on, name prompts. The models already take
  the outcome (`Repo::run`, `Cluster::set_image(.., unlocked)`).
- The protected-env unlock is `ServiceView::unlocked`, per tab, until it closes.
- Current env: `Sidebar::set_env` sets `Services.env`, the sidebar's saved state and
  reloads; `Services::set_env` alone leaves the sidebar stale.
- Toasts: one global `Toasts`; `ToastQueue::push` makes an id that `Toasts::push` drops;
  4 kept, no history.
- Share with: `context_menu::share` needs a window only to raise and focus the target.
- Selection lives in views (terminal scrollback, `Code`, `Rendered`); logs have none.
- Typed events today: `RunnerEvent`, `DeckEvent::Visibility`, `NoteDeleted`, `Services`
  `Rescanned`/`Watches`, `Repo` `Opened`, `OpenFile` `Moved`. The rest only notify:
  `Cluster`, `Ports`, `Sidebar`, `Shell`, `AppSettings`, `EditMode`, `Toasts`, the
  deck's book and layout.
- Logs view: text, level, pause, follow and find all live in the view.

## Steps (one commit each)

1. **Nothing moves the keys but the user.** Every layout operation the API uses keeps
   the window's focused pane (`Window.focused`), unless it removes that pane: opening,
   activating, `open_at` beside, `move_tab`, `move_tab_to`, `move_pane`, `attach`.
   Activating another tab in the pane that holds the keys is refused (the focused view
   would stop drawing and the keys go nowhere). `sync_view_focus` follows only changes
   the user made (the deck says who made a change). OS windows opened for the API
   (detach, a layout switch with detached windows) open with `focus: false`; a
   workspace switch by the API doesn't refocus. The UI keeps its behaviour.
2. **Terminals without a view.** The `Runner` starts a terminal's shell (open and
   start, one function); the view only shows it and gives its size (80×24 until shown,
   then resized). Restoring a layout starts its shells as today. Terminals get a
   `Starter` like runs (the user, or a caller's shell instance).
3. **Models that take ids.** Pins and groups name their list
   (`Sidebar::toggle_pin(list, …)`, groups likewise), not the activity shown. One
   `set_current_env(name)` sets `Services`, the sidebar's state and what's saved. Toasts
   keep their id and a history of the last 50 (`ui.toasts`). Share with's core works
   without a window. The file view opens at a line.
4. **Notes with revisions.** A revision per note, increased by every write. A model
   write reaches an open source editor, which reloads keeping the cursor where it can.
   Notes are watched on disk (a change from outside is a write too).
5. **Answers that come later, jobs, waits.** A handler may answer later: the UI thread
   holds the reply. `job.get`, `job.wait`, `job.cancel`, `job.list`;
   `run.wait{until: exit | output matches, timeout}` (`port listening` comes with step 7),
   `terminal.wait{until: idle | output matches, timeout}`. A wait that times out gives
   the state, the output so far and a job to wait on again. A pending wait whose
   caller's shell ends answers `expired`. `file.read` moves off the UI thread and a
   note's first read is capped (both deferred from P2). Timers exist only while
   something waits.
6. **Events.** `events.subscribe{topics}`, `events.unsubscribe`. Server notifications on
   the same connection, numbered, a bounded queue per connection; overflow: `lagged`,
   and the client reads the state again. Output batched every 100 ms, only while there
   is some. Topics: runs, run output, terminals, layout, tabs, notes, settings, edit
   mode, toasts, jobs, workspace. Models that only notify today get typed events.
   Observers exist only while someone subscribes (R1.3).
7. **Interest handles; the observe methods that scan.** `git.repos`, `git.status`,
   `git.branches`, `port.list`, `port.get`, `leftovers.list` (left for P3 by P1): they
   scan only while a subscription or a one-off read holds an interest.
8. **Methods.**
   - present: `view.open{target, place, activate: false}`, `view.close`, `view.focus`,
     `file.open{path, line}`, `pane.split`, `pane.close`, `pane.collapse`,
     `pane.maximize`, `pane.resize`, `pane.move_tab`, `window.detach`,
     `window.attach`, `window.close`, `layout.switch`, `sidebar.show`,
     `settings.set{scale, theme}`, `workspace.activate`, `workspace.rescan`,
     `app.open_url`, `ui.notify{text, kind}` (labelled with the caller).
   - read: `ui.selection{view}`; observe: `ui.toasts`.
   - run: `terminal.open{dir, place}` without a command (with one it's `type`, P4);
     `terminal.stop`: the caller's own; the user's, or one with a program running,
     `approval_required`. `env.set_current`: a protected env, `approval_required`.
   - organize: `note.create`, `note.write{base_revision}`, `note.append`,
     `note.rename`, `layout.save_as`, `layout.rename`, `layout.keep`, `sidebar.pin`,
     `group.create`, `group.rename`, `group.move`, `group.delete` (keeps its contents).
   - Every call that changes something is audited (R13.8) and told in a toast as in 1c.
9. **Clients.** `devdeck_present` and `devdeck_organize` appear by themselves. The MCP
   client waits for a wait's timeout plus 10 s. The instructions get a line on opening
   and notes. `devdeck-ctl watch <topics>`.

Then the diff review, perf (idle CPU with no subscriber and no wait), docs (R13
checkboxes, ROADMAP) and the commands for the user's manual test.

## Decisions (decided by the user, 2026-11-09)

Each with the recommendation:

- **D1. Keys:** an API call never moves keyboard focus; `activate: true` only makes the
  tab its pane's active one.
- **D2. A terminal before its view:** the runner's `DEFAULT_SIZE` (120×30) until
  shown, then resized.
- **D3. A note the user is editing** (changed in the last 10 s, or unsaved):
  `note.write` and `note.append` answer `approval_required` (the defaults table: ask).
  A revision other than `base_revision`: `conflict`.
- **D4. Events:** pushed on the same connection as notifications (the protocol reserved
  them; clients already skip them). The other way: an `events.next` poll.
- **D5. Waits:** 50 s at most per call, then a job. A job lives 10 min after it ends; 16
  per caller at most.
- **D6. `user.ask`:** dropped (decided by the user, 2026-11-09): Claude asks in its own
  session, never in devdeck.
- **D7. Approvals in P3:** no service and no card (both P4); what would ask answers
  `approval_required`.
- **D8. View state of the logs** (pause, follow, level, find): waits for M3 step 4,
  which moves the logs text out of the view. The matrix highlight isn't blocked by it
  and comes in step 8.
- **D9. Presets and changing permissions in Settings:** P4, with the card.

## Out of scope

The `Approvals` service, its proofs, the protected-env unlock as a token, and the card
(all P4); `type`, `remote` and `destructive` methods; raising an outside program;
the chosen MCP tools (P5); cluster methods (P6).

## States to check visually

- An API open while the user types in a terminal: the keys stay.
- A note edited by the user and written by the API.
- A toast from `ui.notify`, labelled with the caller.
- A terminal opened by the API in a pane not shown.

## Tests

- An API open never moves focus (GPUI test context).
- A shell starts with no view.
- Note revisions, `conflict`, an external write reaching the editor.
- A wait timing out gives what it has; a pending wait whose shell ended: `expired`;
  devdeck quitting with waits pending answers them.
- A slow event reader gets `lagged`; memory stays bounded.
- No timer, observer or scan while nobody waits, subscribes or reads.
- The permission matrix with the new classes; Outside within a terminal.

## Unsure

Settled by the review (checked in the code and gpui-component's source):

- `Deck::sync` builds every tab of the live layout, shown or not (inactive tabs,
  collapsed panes, inactive workspaces' decks), not saved layouts; a terminal's shell
  starts when its view is built, as the user's (`Starter::User`). Step 2 is still needed:
  for the caller as starter, and for R13.1.
- Going to a line: `set_cursor_position` focuses the editor; `set_selected_range(o..o)`
  moves and scrolls without focus (offset from `position_to_offset`).
- Replacing the text: `set_value` and `replace_all` reset the selection, scroll to the
  top and clear undo. Step 4 saves the cursor and restores it clamped; the user's undo
  history is lost on an outside write (or a diff replaces only the changed range).

Still the user's to decide:

- Deleting a layout has no confirmation in the UI, yet the plan calls it destructive;
  adding one is a UI change outside this phase unless the user wants it.

Decided by the user (2026-11-09): P3 stays one phase; the `Approvals` step moves to P4
beside the card, since nothing in P3 uses its proofs through the API. The steps after
it were renumbered (5–9).

## Plan review (subagent), assessed

1. Layout ops other than open and activate move `Window.focused` (`open_at`,
   `move_tab`, `move_tab_to`, `move_pane`, `attach`), so the user's next ctrl+w acts on
   another pane — taken: step 1 keeps the focused pane for every API op.
2. Activating another tab in the pane holding the keys leaves the keys nowhere —
   taken: refused (step 1); added to the visual checks.
3. Detach and a layout switch open OS windows with focus; `workspace.activate`
   refocuses and leaves the caller's scope — taken: `focus: false`, no refocus;
   `workspace.activate` limited to the caller's own workspace (R2.1).
4. Waits, events and selection have no class or ownership rule: a regex wait on the
   user's shell is an oracle over what they typed — taken: a topic or wait condition
   that carries output is class read, and the own-terminal rule (`with_terminal`)
   applies to waits, events and `ui.selection`.
5. The transport can't push (one thread reads, calls, writes) and `Client::call_raw`
   drops events silently — taken: step 6 adds a writer per connection fed by the
   bounded queue, and the client gains a way to read events (for `watch`); events
   arriving during a call are kept, not dropped.
6. `unlocked: bool` and a `pub(crate)` proof constructor are forgeable within the crate
   — taken: the unlock becomes a token type; proofs are made only inside an `approvals`
   module from an answer to a request `Approvals::ask` made; a test checks `api::`
   can't build one. Moved to P4 with the `Approvals` step.
7. A client hanging up mid-wait isn't noticed; one caller can fill the 32 connections —
   taken: the connection's thread watches for hang-up while waiting; caps per caller on
   connections and pending waits; tests for both.
8. The MCP bridge has one connection behind a mutex, so a 50 s wait blocks every other
   call, and a timed-out wait would be retried — taken: waits use their own connection,
   and are never retried.
9. The API's own write would count as the user editing, and the disk watch would see
   devdeck's own atomic writes — taken: the user's edit time comes only from the
   editor's `CodeEvent`; a watch event whose content equals the stored text is ignored.
10. `file.open` could show `~/.ssh/id_rsa`; `app.open_url` could send data out; a flood
    of `ui.notify` evicts devdeck's own toasts — taken: `file.open` has `file.read`'s
    scope and secret rules; `app.open_url` takes an app's URL by id only; `ui.notify`
    is capped in length and rate.
11. `window.close` on the main window quits, and `Deck::close_window` asserts against it
    — taken: refused (quit is destructive, P4).
12. `sidebar.show` clears the filter and selection — taken in part: it clears them only
    when the activity changes (`sidebar.rs` `show`); `sidebar.show` to another activity
    while the filter holds the keys answers `busy`.
13. Rate limits (check 5 of R13.5) are in no step — taken: in step 8, per caller, for
    the methods that act; told in the audit.
14. `run.wait{port listening}` needs the port interest (now step 7) — taken: that
    condition comes with step 7.
15. Terminals the caller opens would still be "not its own" for output and waits —
    taken: own means the caller's shell or a terminal whose starter is the caller.
16. `file.toggle_rendered` and `range` dropped silently; the matrix highlight isn't
    blocked by M3 step 4; ports and git topics missing; no proof for quit or layout
    delete — taken: `file.toggle_rendered` and the matrix highlight go to step 8,
    `range` waits (no use yet, said here); ports and git topics come with step 7;
    quit and layout delete are destructive, P4, with their proofs.
17. Terminals already carry `Run.starter` (always the user); 80×24 contradicts
    `DEFAULT_SIZE` 120×30 — taken: step 2 sets the caller as starter; D2 uses
    `DEFAULT_SIZE`.
18. Toggles undo themselves when called twice; `group.move` is ambiguous — taken: API
    ops take the desired state (`pinned: bool`, `collapsed: bool`, `maximized: bool`);
    `group.move` moves a group, `group.put` puts an item in one.
19. Toast history and `job.list` are in time order — taken: exceptions to R4.1, as the
    audit.
20. An answer that comes later is audited as done too early — taken: the outcome is
    audited when the reply is sent.
21. Waits pending at quit — taken: on quit they answer `unavailable` before the
    listener stops; checking GPUI's quit timeout is part of step 5.

Visual states added: a tab activated in the pane holding the keys (refused); detach
and layout switch by the API while the user types; `sidebar.show` while the user types
in the filter; a note in source view written by the API (cursor kept, undo lost);
`file.open` at a line in a pane not drawn, then drawn; a long or multi-line
`ui.notify` (R4.4); an API close or rename during a drag, a Send-to pick or a name
prompt; an API-opened terminal in the Terminals list.

Tests added: hang-up mid-wait; per-caller connection and wait caps; events arriving
during a call kept; the notes watch ignoring devdeck's own write; going to a line
without focus; every API layout op keeping `Window.focused`; topic and wait classes in
the permission matrix.

## Step 1, as built (2026-11-09)

Touches no OS code (R11.6): the layout model, the deck and GPUI focus only.

- `Layout::in_background(target, op)`: all or nothing; each window keeps its focused pane
  and each pane its active tab unless `op` took them away; only `target` may show
  another tab. `Layout::shown_in(window)` (also used by the deck's visibility).
- `Deck::apply_quietly(keys, target, op)`: a change the user didn't make. Refused
  (`Quiet::Keys`) when a pane holding the keys would stop being its window's focused
  pane, show another tab, or be hidden (collapsed, maximized over); layout refusals are
  the caller's (`Quiet::Refused`), never toasted. Windows it opens ask for no focus.
- `window_view::keys(deck)`: where the keys are, per window: the pane whose view has
  them, or the focused pane when they're on the pane area; and where they go back to
  (Settings open, another workspace shown), recorded when the focus was put aside.
- Built differently from the plan: instead of `sync_view_focus` telling the user's
  changes from others, the panes holding the keys are guarded, so a quiet change can't
  give `sync_view_focus` anything to follow there. "A workspace switch by the API
  doesn't refocus" waits for `workspace.activate` (step 8): a switch hides every pane of
  the workspace left, so it needs its own rule there.
- Linux: GPUI 0.3.11's Linux backends ignore `WindowOptions::focus`, so a detached
  window opened quietly may still take the keyboard there; step 8 decides whether
  `window.detach` and a `layout.switch` that opens windows are refused on Linux.

Diff review (subagent), assessed:
1. Moving the keys' view within its window moved the keys (`sync_view_focus` followed
   the restored focused pane's new tab) — taken: the guard holds the pane, its tab and
   its focus, not just the view being shown.
2. Keys on the pane area itself (a view that takes none) were unseen — taken.
3. Focus put aside for Settings or another workspace wasn't guarded — taken: recorded
   with the pane when put aside, and guarded.
4. `focus: false` does nothing on Linux, unrecorded — taken: noted here; step 8 decides.
5. `keys` panics inside an update of the deck and answers `None` during any window's
   update — taken: said in its doc; windows of other decks are read without an update.
6. Two layout clones per quiet change — rejected: a layout is small, and
   `in_background` stays all or nothing for its other callers.
7. A quiet change cleared the user's `last_error` — taken: it doesn't any more.
10. No test for moving the keys' view within its window, keys on the pane area, or
    behind Settings — taken: added. Keys in a detached window and in an inactive
    workspace have no test yet (the test shell has one workspace).
11. Nothing checks `focus: false` — rejected: GPUI's test platform doesn't record it.
14. Three near-identical names — taken: `opening_quietly`, `open_unfocused`.

## Step 2, as built (2026-11-09)

Touches no OS code (R11.6): the shell is spawned by the existing driver, from another
caller.

- `Runner::open_shell(n, dir, env, starter)`: opens the terminal and starts its shell now,
  or once the tools are known (`set_catalog`); while it waits, who asked first keeps it.
  A terminal the runner already ran isn't started again (only ⏎); one that can't run
  stays not started; a waiting one forgotten meanwhile (R6.12) is dropped.
- A terminal tab, when built, calls it as the user; `RunView` never starts a shell (its
  `start_if_restoring` is for runs restored as running only).
- The size stays the runner's `DEFAULT_SIZE` until a view measures, as before (review
  point 17).

Diff review: the subagent was refused by Claude Code's permission classifier; reviewed by
the lead instead: the start now runs inside the deck's update (`Deck::sync` builds the
tab), and the runner's events and notifications reach observers after it, so nothing
re-enters the deck; the ports check updates `Ports`, not the deck; env and size are read
when the tab is built, as before. Found in testing: a stopped terminal no tab shows is
forgotten (R6.12), so opening it again rightly starts a new shell; the test checks a
running one instead.

## Step 3, as built (2026-11-09)

Touches no OS code (R11.6).

- Pins and groups name their list (`Activity`), shown or not: `Sidebar::set_pinned(list,
  key, pinned)` (the state wanted: twice changes nothing more; terminals refused), and
  `create_group`, `put`, `rename_group`, `move_group`, `delete_group` take the list; the
  UI passes the one it shows. The selection follows a moved group only in the shown list.
- `Sidebar::set_env(name)` stays the one way to set the current env (services, list,
  saved together) and now refuses an unknown env.
- Toasts: `Toasts::push` returns the id; `ToastQueue::history` keeps the last 50 told
  (id, kind, text), expired or not, in time order (an exception to R4.1).
- `context_menu::paste(runner, terminal, text)`: Share with's paste without a window
  (one block, control characters removed, no Enter, bracketed paste only); `share` adds
  showing the terminal.
- `FileView::go_to_line(line)`: the caret at the line's start, scrolled to, once the text
  is read and the source drawn (`Code::go_to_line` uses `set_selected_range`, which
  scrolls without taking focus); a Markdown file shown rendered keeps the line until its
  source shows. `Code::caret` reads it back.

Review: by the lead (the subagent refused in step 2 wasn't tried again).

## Step 4, as built (2026-11-09)

Touches no OS code (R11.6): the notes' folder joins the watcher's existing route for the
files views show.

- `Notes::revision(id)`: raised by every change of a note's text (0 until the first).
- `Notes::set_text` is the user's typing (records `typed_at`, for D3); `Notes::write` is
  any other writer (the control API): written soon, its new revision returned, told with
  `NoteWritten`. Unknown or unreadable notes are refused.
- An open source editor takes a `NoteWritten` at its next draw: `Code::set_text` now
  keeps the caret (moved back inside a shorter text) as well as the scroll, for file views
  too; the editor's undo history goes with the old text (gpui-component has no way to
  keep it). Rendered views read the model on every draw already.
- Disk: `Services::file_watch` adds the notes' folder; `Notes::disk_changed` takes a read
  note whose file now says something else (another program's change: a revision, told
  to editors). devdeck's own atomic writes read back the same: nothing. A note whose
  text waits to be written keeps it and writes over the file.
- Not done: the folder is only watched once it exists (made with the first note); the
  watcher picks it up at its next look at the watched folders (any rescan).

Review: by the lead.

## Step 5, as built (2026-11-09)

API level 4. OS code (R11.6): the connection's hang-up check uses POSIX `recv` with
`MSG_PEEK | MSG_DONTWAIT` through `libc` in `api/transport.rs` (std's `peek` isn't
stable): Linux and macOS both; to list in `docs/PLATFORMS.md`'s inventory until
`devdeck-sys` exists. `file.read`'s existing `/proc/self/fd` check moved to a background
thread, unchanged.

- Answers that come later: a handler gives a task (`dispatch::Answer::Later`); the UI
  thread holds the reply, one per connection (`ApiState::later`), dropped when the
  client hangs up (the connection's thread looks every 250 ms while it waits), when the
  connection closes, or when devdeck quits (the call answers `unavailable`). The caller's
  shell ending ends it with `expired` (looked at every 100 ms, on the connection's side:
  `api::answer`). The audit records the outcome when it's known.
- `run.wait{until: exit | {output: PATTERN}}`, `terminal.wait{until: idle | exit |
  {output}}` (the caller's own terminal only), class read: they look every 100 ms while
  someone waits, nothing otherwise; 30 s by default, 50 at most. Lines are looked at
  once: a job resumes after the last line looked at. A restart, a resize or a
  full-screen program goes on from what shows. Time up: `done: false`, the state, the
  output since it began, and a job.
- Jobs: resumable waits, not background work. `job.get` (looks now), `job.wait`,
  `job.cancel`, `job.list` (oldest first: an exception to R4.1); a caller's own (its
  shell instance), kept 10 minutes after they began, 16 a caller.
- Caps: 4 waits at once a caller (`busy`); 8 connections a caller (refused with `busy`,
  audited once a minute).
- `file.read` reads in the background; a note larger than 2 MB isn't read (shown as
  unreadable, for the UI too).
- MCP: a wait goes on a connection of its own, waits for its time and 10 s more, and is
  never made again; the instructions say how to wait.

Review: by the lead.

## Step 6, as built (2026-11-10)

Touches no new OS code (R11.6).

- `events.subscribe{topics}` / `events.unsubscribe`, class observe, API level 4. An event
  says what changed, never its content (built differently from the plan, which had output
  as read): `runs {app, cmd, state, exit_code}`, `terminals {n, state, exit_code}`,
  `output {app, cmd}` or `{n}` (the caller's own terminal only), `layout {}`,
  `notes {id, revision}` or `{id, deleted}`, `workspace {shown}`, `settings {}`,
  `edit_mode {on}`, `toasts {id, kind}`. No `jobs` topic: a job doesn't run between
  waits. Git and ports topics come with step 7.
- Notifications `event {seq, topic, data}`, numbered per connection from 1;
  `events.lagged {seq}` names the first one dropped.
- Transport: each connection has a writer thread fed by one bounded queue
  (`OUTBOX` = 256 lines) for answers (blocking) and events (`try_send`, never blocking the
  UI thread); a line not written within 10 s ends the connection. The client
  (`devdeck-api`) keeps notifications received during a call (1024 at most) for
  `next_notification`.
- The hub watches every workspace's runner, deck and notes, the shell, edit mode, the
  settings and the toasts only while some connection follows a topic, and goes with
  the last; changes are gathered and sent every 100 ms at most, the last of each thing
  winning. A caller hears its own workspace only; a caller whose shell ended follows
  nothing at the next send.

Diff review (subagent), assessed:
1. A subscriber whose shell ended kept hearing events and kept the hub — taken: dropped
   at the next send (`ApiState::alive`, which tests set: the demo's shells have no
   process).
2. A client that stops reading could block its connection's thread — taken: a 10 s write
   timeout ends it.
3. The client's kept notifications had no bound — taken: 1024, the oldest go.
4. Sends go in (topic, key) order, not time order — kept: the last of each thing wins;
   `seq` orders what's sent.
5. Test gaps — taken: a terminal's output reaches that terminal only; closing a
   connection and an ended shell stop the watching. Left: the workspace, terminals,
   edit mode, settings and layout topics end to end, and `events.lagged` through a real
   connection (its queue logic has a unit test).

## Step 7 plan (2026-11-10): interest handles, the observe methods that scan

What runs today only while a view wants it (R1.3) runs for the API the same way: a
handle held by a one-off read until it has its answer, or by an `events` subscription
while it lasts. No new OS code (R11.6): the scans are the ones views use (git through
core's runner, ports through `core::ports`, leftovers through `GroupLog`).

Methods (class observe, API level 4, a terminal caller's own workspace):
- `git.repos{workspace?}`: the repositories of the workspace's apps (roots only of known
  apps, never above a workspace folder): `{root, apps}` by root. Finding roots stats
  folders: done in the background (an answer that comes later).
- `git.status{root}`: head (branch, detached at, unborn), upstream (name, ahead, behind,
  gone), what's under way (rebase…), the changes (path, from, kind, staged; paths are
  untrusted; at most 500, `truncated`). A one-off full read: `Repo::look` held until the
  state is ready, 10 s at most (`unavailable` past it).
- `git.branches{root, remote?}`: name, upstream, commit, subject and author (untrusted),
  local first then remote ones when asked. `Repo::glance` held until ready.
- `port.list{workspace?}`: `Services::ports` (port, holder, command, the run when ours),
  by port. A one-off read starts a scan when none runs (`Ports::interest`) and answers
  after the next scan, 5 s at most; the demo (no machine scan) answers at once.
- `port.get{port}`: one port, its holder named even when it isn't ours (the port is
  watched for the scan, as a port view's is).
- `leftovers.list`: what a previous devdeck left running (`GroupLog::leftovers`, in the
  background): label and pids. Stopping them stays the user's (P4, destructive).

Interests:
- `Ports::interest() -> PortsInterest` (an `Rc` token, as `Repo`'s `Looking`): scanning
  is wanted while one lives; with ports to watch (`port.get`) they join the scan's.
- Topics: `ports {}` when a scan's result changes (the subscription holds a
  `PortsInterest` per workspace with subscribers); `git {root}` when a repository's state
  changes — for repositories something already holds (a view, a one-off read): a
  subscription doesn't start reading every repository.

Tests (`tests/app/api_git_ports.rs`, a module of `tests/app/main.rs`): roots of the
workspace's apps only; a status read of a throwaway repository (as the git panel's tests
make one), branches with remote ones on demand; reads that end release their handle (no
read or scan after); the demo's ports at once; a port interest starts a scan and its end
lets scanning stop; leftovers on an empty log; a `ports` subscription keeps scanning, an
unsubscribe stops it; another workspace's root is `not_found`.

Out of scope: git writes, stopping a port's holder or leftovers (P4); cluster (P6).

Plan review (subagent), assessed:
1. `port.get` named any process with its full command line (tokens) and could probe any
   port, another workspace's runs showing as foreign — taken: only ports the workspace's
   runs listen on or its commands declare (else `not_found`); a holder that isn't one of
   its runs is named by its program only, no pid, no arguments; untrusted.
2. Leftovers have no workspace (one log for all) — taken: `leftovers.list` moves to P4
   with `leftovers.stop`, once entries say whose they are.
3. `git.status` would list untracked secret-named files (`.env.local`, `id_rsa`) —
   taken: `file.read`'s secret rules hide them (counted in `hidden`); paths, branch
   names and subjects are untrusted.
4. A port interest with nothing to scan scans nothing, an unchanged scan wakes no one,
   `ensure` isn't called — taken: an interest makes scanning wanted only with runs or
   watched ports, calls `ensure`, and the read waits for the scan count to move.
5. Watched ports keyed by view — taken: interests are tokens of their own (with their
   ports), released when dropped (an answer, a hang-up, the end of a subscription).
6. Reads that continue: a re-read after release, git's own time limit, no per-caller
   bound — taken: one-off reads count against the caller's waits (4 at once); at most one
   read after release and git's own 10 s limit stay (said in the as-built note).
7. "Ready" — taken: a status waits for a full read with none in flight; an error or a
   repository that can't be opened answers at once.
8. Roots — taken: the folders `file.read` allows (no hidden app, canonical, never `/` or
   home), and a root never above the app's workspace folder (an app outside every folder:
   its own folder at most); `git.status` and `git.branches` take a root of that set only,
   computed in the background; the demo has none; unknown and another workspace's root
   are the same `not_found`.
9. Branches unbounded — taken: `limit` (200, 1000 at most) and `truncated`; no worktree
   paths; author names only, untrusted.
10. The `git` topic needs repositories to say what changed (they only redraw) — taken:
    the `git` topic moves to P4 with git writes; `ports` events are their workspace's.
11. Slow reads — kept as observe answers that come later within their bound (10 s for git,
    5 s for ports), no job; catalog entries at level 4.
Tests left out: a foreign holder's name (the demo has no machine scan; the program-only
rule has a unit test), an inactive-workspace subscription.

## Step 7, as built (2026-11-10)

No new OS code (R11.6). API level 4, class observe, the caller's own workspace.

- `git.repos`, `git.status{root}`, `git.branches{root, remote?, limit?}`
  (`api/gitports.rs`): roots found in the background from the folders `file.read`
  allows, never above the app's workspace folder, never `/` or the home folder (a root
  checked as well as the app's folder); a root not of that set is `not_found`. A status
  holds `Repo::look` (a full read), branches `Repo::glance`, until the state is read and
  no read is in flight, 10 s at most; a failed read answers at once, unless a new read is
  in flight. Changes: 500 at most, secret-named paths counted in `hidden`; branches 200 by
  default, 1000 at most, `truncated`; author names only; all of it untrusted.
- `port.list`, `port.get{port}`: the ports the workspace's commands declare and its runs
  listen on, by port, and who holds each: a run's title, another program by its name
  only (`holder_name`). A read holds a `Ports::interest` with its ports watched and
  answers after the next scan (the one after when a scan was under way), 5 s at most, or
  at once when there's nothing to scan (the demo).
- `Ports::interest(ports)`: scanning is wanted while one lives with something to scan;
  watched ports join the scan's.
- One-off reads count among the caller's 4 waits (`waits::hold`).
- Topic `ports {}`: a workspace's scan changed; a follower keeps its workspace scanning
  (while there's something to scan).
- Not here (plan review): `leftovers.list` (P4), the `git` topic (P4).
- Found in the full suite: the client failed with a broken pipe when devdeck refused a
  connection before reading its request (a test, flaky under load); the client now reads
  the refusal devdeck wrote before closing.

Diff review (subagent), assessed:
1. Promised tests missing — taken: the holder rule (unit), an interest scans its ports
   while held and stops when dropped, one-off reads count among the waits. Left: a `ports`
   follower keeping a real workspace scanning (the demo has no scan; the hub holds the
   same `Ports::interest` the test covers).
2. A root could be `/` or the home folder — taken: checked like the app's folder.
3. A stale read error answered while a new read was in flight — taken.
4. A scan under way could answer without the newly watched port's holder — taken: the
   read waits for the scan after it.

## Step 8 plan (2026-11-10): the methods

Built in two commits under this one plan: 8a present and read, 8b run and organize. No
new OS code (R11.6). API level 4; a terminal caller's own workspace (R2.1). Every call
that changes something is audited (it is already, by class) and told in a toast, one per
caller every 2 s at most (as in 1c).

Rules for all of them:
- **Taking the user's keys asks the user.** A change `Deck::apply_quietly` refuses with
  `Quiet::Keys`, `window.detach` (a new window may take the keyboard: on Linux GPUI can't
  open one without focus, and nothing in our code branches on the OS), a `layout.switch`
  that opens detached windows or hides the panes holding the keys, and
  `workspace.activate` while another workspace is shown answer `approval_required` (P4's
  card can allow them). This settles the step 1 open points without OS-specific code.
- Panes, windows and splits are named by the ids `ui.get` gives (live layout only);
  views by the typed `View` of `ui.get` (`{kind: "file", path}`…).
- Toggles are states asked for (`collapsed: bool`, `maximized: bool`, `pinned: bool`,
  `keep: bool`, `rendered: bool`): asking twice changes nothing more.
- Rate limit (R13.5 check 5): calls of the present, organize and run classes, 30 per
  10 s per caller, then `rate_limited`; `ui.notify` 1 per 2 s on top.

### 8a, present and read

- `view.open{view, pane?, edge?, activate?}`: an app, a run, a file (with `file.read`'s
  scope and secret rules), a note, or a port of the workspace; other kinds `invalid`
  (terminals open through `terminal.open`, repositories later). Into `pane` (the main
  window's focused one when left out) or beside it on `edge`; `activate` (false) makes it
  that pane's active tab (`target` of `apply_quietly`). Already open: moved only when a
  place is given, else activated when asked.
- `view.close{view}`, `view.focus{view}` (its pane's active tab; never the keyboard).
- `file.open{path, line?, pane?}`: `view.open` of the file, then `FileView::go_to_line`.
- `file.rendered{view, rendered}`: a Markdown file's or note's rendered or source form.
- `pane.split{pane, edge}` (its active tab to a new pane on `edge`; a pane of one tab:
  `invalid`), `pane.close{pane}` (its tabs close; a terminal's shell goes on, R6.12),
  `pane.collapse{pane, collapsed}`, `pane.maximize{pane, maximized}`,
  `pane.resize{split, index, fraction}`, `pane.move_tab{view, pane, at?, edge?}`.
- `window.detach{view}` (asks, see above), `window.attach{window, pane}`,
  `window.close{window}` (a detached one; the main window: `invalid`, quitting is P4).
- `layout.switch{id}` (asks when it would take the keys or open windows).
- `sidebar.show{activity}` (the sidebar and that activity; another activity while the
  user types in the filter: `busy`).
- `settings.set{scale?, theme?}` (a known theme; scale within the UI's bounds).
- `workspace.activate{}`: the caller's own workspace; already shown: nothing; else asks.
  `workspace.rescan{}`.
- `app.open_url{app, url}`: one of the app's own URLs, by its name or index, opened in
  the browser as the app view's link does; never a free URL.
- `ui.notify{text, kind}`: a toast labelled with the caller, `info` or `warn` (never
  `err`, which looks like devdeck's own), one line, 200 characters at most.
- `ui.toasts{}` (observe): the toast history (id, kind, text untrusted), oldest first.
- `ui.selection{view}` (read): the selected text of a view; a terminal's the caller's own
  only (`with_terminal`'s rule); untrusted, bounded like `file.read`.

### 8b, run and organize

- `terminal.open{dir?, pane?, edge?}`: a shell started as the caller (`Runner::open_shell`
  with `Starter::Caller`) in `dir` (one of the workspace's apps' folders or folders; the
  caller's own terminal's folder when left out), the next number; a tab only when a
  place is given (else the Terminals list shows it). With a command it is `type` (P4).
- Own terminals: the caller's shell, or one it started (`Run.starter`); waits, output,
  search, screen and selection follow (review 15).
- `terminal.stop{n}`: one it started, with no program in the foreground; the user's,
  another caller's, or one running a program: `approval_required`.
- `env.set_current{env}`: `Sidebar::set_env`; a protected env: `approval_required`.
- `note.create{app?, name}` (its id and revision), `note.write{id, text, base_revision}`,
  `note.append{id, text, base_revision?}`, `note.rename{id, name}`: a note the user typed
  in within 10 s: `approval_required` (D3); `base_revision` not the note's: `conflict`;
  64 KB of text at most per call.
- `layout.save_as{name}`, `layout.rename{id, name}`, `layout.keep{view, keep}`.
- `sidebar.pin{list, key, pinned}`, `group.create{list, parent?, name}`,
  `group.rename{list, path, name}`, `group.move{list, path, parent?}`,
  `group.put{list, node, group?}`, `group.delete{list, path}` (its contents move up).

### Tests

A module per area of `tests/app/` (`api_present.rs`, `api_organize.rs`), on the demo:
every method's happy path; the keys guard answering `approval_required` (a view opened in
the pane holding the keys, a layout switch, `window.detach`, `workspace.activate` while
another workspace shows); a file outside the scope or secret-named refused by
`view.open` and `file.open`; `app.open_url` with an unknown name; `ui.notify` cut and
rate-limited; the rate limit; `terminal.open` without a place (no tab, listed, started as
the caller), then its output readable as the caller's own; `terminal.stop` of the user's
terminal asks; a protected env asks; notes: conflict, the user typing asks, append; pins
and groups by list, twice the same; another workspace's anything `not_found`.

States to check visually (8a): a view opened beside the pane holding the keys; a toast
from `ui.notify`, long text truncated (R4.4); the Terminals list with a terminal opened
by the API and no tab.

Out of scope: the card and what it approves, `type` (commands, input, paste), git
writes, deletes, quitting, cluster (P4–P6); `user.ask` (dropped); `devdeck_present` and
`devdeck_organize` MCP tools (step 9).

Plan review (subagent), assessed:
1. `layout.keep`: "keep in every layout" was removed (DESIGN, 2026-11-09) — taken:
   dropped (the inventory line above is out of date on it).
2. Bare pane ids repeat across layouts; terminals need their uid — taken: panes,
   windows and splits are named with the layout id (`{layout, pane}`), `conflict` when
   it isn't the live layout; `terminal.stop{n, uid}`.
3. The matrix highlight — deferred (later, with the logs' view state, D8).
4. `toggle_rendered` takes the window and moves focus — taken: a `set_rendered(bool)`
   without focus for the file and note views.
5. `layout.switch` can't tell per target whether the keys move — taken: it asks whenever
   a pane holds the keys or either layout has a detached window; the book changes only
   once allowed.
6. `Deck::open_terminal` isn't quiet and starts as the user — taken: a quiet
   `open_terminal_as(n, dir, place, starter, keys)`, the runner told first.
7. A terminal without a tab is forgotten once not running — taken: `terminal.open`
   always gives it a tab, in the background unless a place is asked.
8. `app.open_url`: no app-view link exists, discovered URLs come from files an agent
   can edit, and a browser takes the keyboard — taken: moved to P4 (it asks there).
9. `view.open` of a file must check its scope off the UI thread; missing targets make
   junk tabs — taken: an answer that comes later, the canonical path in the view; apps
   (not hidden), notes and ports must exist.
10. `ui.selection`: only terminals have one; scope and secrets — deferred to P5 with
    selection in the file and note views.
11. `sidebar.show`'s "user types in the filter" needs a window check that doesn't exist
    — deferred (later).
12. Model functions that toast instead of returning errors — taken: the handlers check
    first (note exists, name not empty, theme known, group exists).
13. Pane states from toggles — taken: a state already as asked changes nothing.
14. `workspace.activate` would always ask until P4 — taken: moved to P4.
15. No cap on terminals — taken: 4 live terminals opened by one caller.
16. A shell under a protected current env — taken: `terminal.open` asks then.
17. Note growth — taken: a write or append past `MAX_NOTE` refused; names 100
    characters; 500 notes a workspace; `app` must be one of the workspace's (or none).
18. `ui.toasts` is content — taken: class read.
19. `ui.notify` and the per-caller notice — taken: its own toast, control and
    right-to-left characters removed.
20. Closing the tab of a terminal not running forgets it (with its output) — taken:
    `view.close` / `pane.close` of such a tab asks.
21. Caps — taken: 200 pins a list, 100 groups a list, 50 layouts, a rescan per 5 s.
Moved with it: `window.detach` (it always asked) goes to P4 too.

## Step 8a, as built (2026-11-10)

No new OS code (R11.6). API level 4, class present (`ui.toasts`: read).

- `api/present.rs`: `view.open` (an app, a run, a file, a note, a port of the workspace;
  hidden apps and missing targets refused; in the background unless `activate`),
  `view.close`, `view.focus`, `file.open{path, line?}` (shows the file; `file.read`'s
  scope and secret rules, checked in the background; the real path in the view),
  `file.rendered` (`set_rendered` on the file and note views, no focus; refused while the
  user types in the view), `pane.split/close/collapse/maximize/resize/move_tab`,
  `window.attach/close` (never the main window), `layout.switch`, `settings.set`,
  `workspace.rescan` (once per 5 s a workspace), `ui.notify` (one clean line of 200
  characters at most, labelled, one per 2 s a caller, its own toast), `ui.toasts`.
- Every layout change through `Deck::apply_quietly`; its `Quiet::Keys` answers
  `approval_required`, as does a layout switch while a pane holds the keys or a detached
  window is involved, and closing the tab of a terminal that isn't running.
- Panes, windows and splits named with the live layout's id (`conflict` otherwise);
  `ui.get` gives split ids.
- Rate limit in the dispatcher: 30 calls of the present, organize and run classes per
  10 s a caller, then `rate_limited`.
- Found in the suite: the transport's refusal was audited after the client heard it (a
  test raced); now before.

Diff review: the subagent was refused by Claude Code's permission classifier; reviewed by
the lead, the user's choice (2026-11-10). Found: `view.open` of a file always showed it
(ignoring `activate`) — fixed.
