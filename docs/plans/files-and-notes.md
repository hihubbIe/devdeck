# Files and notes

Asked 2026-11-08 (user). Comes before the Claude control (MCP), which builds on it.
Five steps, one commit each.

## What

- **File view.** Any text file dropped on a pane opens there in a tab, view only, with
  syntax highlighting like Zed's. A Markdown file can switch between its source and a
  rendered view.
- **Notes.** Markdown notes made from an app's view, kept by devdeck (never in the
  project), linked to the app, saved by themselves. Closing the tab loses nothing: the
  app's view lists them and opens them again. A note switches between editing its source
  and a rendered view with an icon in its tab's header.
- Nothing else edits files: no file explorer, no HTML, no code editing.

## Decisions (user, 2026-11-08)

- Files are view only. Notes are the only thing devdeck edits.
- A file opens by dropping it on a pane (later also through the MCP).
- Notes are listed in the app's view, for now.
- Source and rendered view in the same tab, switched by an icon in the header.
- Components: gpui-component 0.9.0, which pins `gpui-pre =0.3.11` like us: `Editor`
  (read only for files, editable for notes) with tree-sitter highlighting, and
  `TextView::markdown` for the rendered view.
- "No git or editor features" goes: git has its panel (R6.10), and the editor rule becomes
  "files are view only, except notes".

## Defaults taken in this plan (to confirm)

- **Notes live in `$XDG_DATA_HOME/devdeck` (`~/.local/share/devdeck/notes/`)**, not with the
  state: they are what you write, and state folders get cleared. R10.1 gains this folder.
- **A dropped file already open moves to where it's dropped**, as a list row dropped on a
  pane does today (`sidebar::open_at`), rather than staying where it is.
- **The notes list is one alphabetical list, without Pinned + All**: it sits inside the
  app view like its commands, not in the sidebar. Written down as an exception to R4.2.
- A dropped file that isn't text (a NUL byte in its first 8 KB, or not UTF-8) or is over
  10 MB opens a message instead, in the tab, naming why. Over 1 MB, or with a line of
  5 000 characters or more, it shows as plain text (step 1's measurements).
- A dropped folder is refused with a toast; dropped with files, the files open and the
  toast names what was left. A path that isn't valid UTF-8 is refused the same way.
- Several files dropped at once open one tab each, in drop order, all in the same place:
  on an edge, the first one splits and the others join its new pane.
- Code doesn't wrap (R4.4) and scrolls sideways; the rendered Markdown wraps, and so does a
  note's source (prose): the views opt in, as R4.4 allows.
- A note has a name (asked when made, "notes" if left empty), renamed and deleted from its
  row's menu; delete asks first. Same names are allowed, ordered by id after the name.
- An app that disappears keeps its notes on disk (like pins), unseen until it comes back; an
  open note tab keeps working.

## Requirements (new section in REQUIREMENTS.md)

12. Files and notes
- **R12.1** A file view (`file/<path>`): any text file, view only, highlighted by its
  language (from the extension, plain text otherwise), line numbers, select and copy,
  find. It follows the file while shown: changed on disk, it reloads keeping its scroll;
  deleted, it says so and keeps the tab; back, it shows it again.
- **R12.2** A file dropped from outside (a file manager) onto a pane opens there: in the
  pane, or at an edge to split it, like a list row (R3.4). A file already open moves there.
- **R12.3** Notes: Markdown notes belonging to an app, kept in devdeck's data folder, made,
  opened, renamed and deleted from the app's view. A note is saved by itself while it's
  edited (no save key, no unsaved marker) and whenever devdeck saves its state or quits.
- **R12.4** A Markdown file or a note shows its source or rendered, switched by an icon in
  its header (or tab strip), its tab menu, the palette and a key. A note opens rendered
  when it has text, as source when empty. What's shown is remembered per tab.

Also: R3.3 examples gain "file, note"; R4.2 notes its one exception; R10.1 the data folder;
CLAUDE.md's "No git or editor features (R6.10)" becomes "Files are view only, notes aside
(R12)".

## Step 1 · Spike, then the docs

Throwaway, in a scratch binary (`crates/devdeck-app/src/bin/editor_spike.rs`, deleted at the
end of the step). Answers, written in DESIGN's libraries table and decision log:

- Builds in our workspace with only these grammars: rust, go, typescript, tsx, javascript,
  json, yaml, toml, markdown, markdown-inline, bash, css, html, python, make, diff, sql
  (gpui-component has no default features). One version of `gpui-pre` in `Cargo.lock`.
- `EditorState` (in `gpui-base`): its key context, its own bindings (compared with our
  registry: ctrl-k, ctrl-/, ctrl-w, Esc…), `set_text`, read only really read only (typing,
  paste, cut, drop, undo, IME do nothing).
- Does it work without gpui-component's `Root` and without `gpui_component::init`? `init`
  binds keys outside our registry (dock, list, table, command) and, in debug builds,
  `ctrl-shift-i` to its inspector, and sets its theme to Light. Find the smallest init
  that works (its theme and the input's bindings only, or our own copy of those bindings).
- Theme: filled from ours with `Theme::update` (never `Theme::change` or
  `sync_system_appearance`), highlight colours from our tokens, our mono font and size
  from the UI scale; changes apply at once.
- Its icons: the find bar uses gpui-component's `IconName`, which our `Assets` doesn't
  serve. Map them with `set_icon_resolver` to our Lucide icons, or find out the find bar
  can be replaced.
- Its right-click menu: replaced by ours through `.context_menu(...)` (R5.2, R5.7).
- Scroll: one axis per gesture (R9.6)? If not, can it be wrapped?
- `TextView::markdown`: our font, task lists, tables, code blocks highlighted, links.
- Cost on a release build: binary size before and after, RSS idle with no editor, with a
  10 000-line file, a 10 MB file and a one-line 2 MB minified file open, and CPU idle with
  each shown (should stay at zero).

Outcome: go on, or stop and come back to the user with what failed. The step's commit is
the docs only: REQUIREMENTS (section 12 and the lines above), DESIGN (library, decisions),
ROADMAP (this, then the Claude control), CLAUDE.md (the rule line).

## Step 2 · Components in `devdeck-ui`

Views draw with `devdeck-ui` only (R9.3), so gpui-component stays inside it.

- `devdeck-ui/src/code.rs`:
  - `code_view(state)`: a read-only editor (line numbers, find, no wrap).
  - `note_editor(state)`: an editable editor, Markdown, soft wrap, no line numbers.
  - `markdown_view(id, text)`: the rendered view.
  - `CodeState`: wraps gpui-component's state, with `set_text` (keeps scroll and selection
    where it can), `text`, `language_for(path)`, and a change event.
  - Our context menu on both editors; the find bar in our look (its icons mapped).
- Theme: `devdeck-ui` gets a function copying our theme into gpui-component's;
  `app_settings::apply_theme` calls it on every theme or scale change, and the start and
  the test host (`tests/common`) call it once.
- Keys: the note editor has its own key context. Only the keys it takes as text go off
  for devdeck inside it, as `registry::terminal_takes` does for terminals, so ctrl+w,
  ctrl+tab, alt+arrows, send and detach still work in a note. The read-only view takes no
  text keys and stays outside any guard. Esc in the note editor leaves it to its pane,
  like `text_input::Cancel` does.
- Tests (`crates/devdeck-ui/tests/code.rs`): read only refuses typing, paste and cut; copy
  works; find; the change event fires on typing in the note editor only; `set_text` keeps
  the scroll; language from extension (unknown → plain); markdown renders a task list and
  a code block; one-axis scroll; our menu on right-click; theme and scale changes reach it.

## Step 3 · Engine: drops, view actions, live titles

What the file view and notes need from the engine, made generic.

- **External drops**: the drop zones and the header or strip take `ExternalPaths` like
  `DraggedListItem` (into the pane or at an edge), with the rules above. GPUI turns a drop
  from outside into an `ExternalPaths` drag on Wayland and X11, so the zones show.
- **View actions in the header**: a view publishes the actions it has right now (id,
  icon, label, on or off) through the registry, not a closure of its own. The header (or
  the strip, for the active tab), the tab menu and the palette all read that one list
  (R5.1). A click acts on that pane's view, even when another pane has the focus. A
  view's change notifies its deck, so the header redraws (an icon showing source or
  rendered stays right).
- **Live titles**: `views::title` takes `&App` and asks the view's workspace when the
  address alone can't say (a note's name). Every caller (strip, header, rail, "Send to…",
  places, Terminals list) passes it.
- With no workspace, a drop opens nothing and toasts why.
- Tests (`crates/devdeck-app/tests/drops.rs`, `view_actions.rs`): `FileDropEvent` into a
  pane, on an edge, two files on an edge (one split), a folder (toast), files and a folder
  (files open, toast), an open file moves; a view action in the header acts on its
  unfocused pane; the header redraws when the view's action changes; a renamed title shows
  everywhere.

## Step 4 · File view

- `core::view_addr`: `File { path }` (`file/<escaped absolute path>`); title: the file name,
  full path on hover.
- `views/file_view.rs`: reads the file off the UI thread; text, too big, not text, missing,
  unreadable each have their state. Saves its scroll and, for `.md`, source or rendered
  (`KeepsState`).
- Following the file: its folder joins the watcher's `wanted` set with its own route in
  `changed` (as git dirs have), so a rescan keeps it. It reloads only while the view is
  shown, at most once a second (a growing log can't keep the CPU busy, R1.3), and once on
  coming back on screen if it changed meanwhile. A removed folder makes the file
  "missing"; the watch comes back with it.
- Action "Toggle source / rendered" (`ctrl+shift+e`, checked free in the registry and
  against the editor's bindings in the spike), published in step 3's way, Markdown only.
- Tests (`crates/devdeck-app/tests/files.rs`): too big, binary, missing, unreadable;
  changed on disk reloads while shown, not while hidden, then on showing; deleted then
  back; its folder removed then back; toggle by button and key; state survives a restart;
  a layout with a `file/` tab whose file is gone.

## Step 5 · Notes

- `core::notes`: a workspace's notes in `<data>/workspaces/<id>/notes/`: `index.json` (id,
  app, name, ordered by name then id) and one `<id>.md` per note. Ids are UUIDs (names are
  labels, as for layouts). Writes are atomic; on make, the `.md` is written before the
  index, on delete the index before removing the `.md`, so a crash leaves at most an
  unlisted `.md` (kept, ignored) and never a listed note without its file (shown empty if
  it happens anyway). Unit tests.
- A `Notes` entity per workspace in `Services` owns the notes' text and what's unsaved.
  Views only edit it, so a tab closed, closed by "close others", a layout switch, a
  detached window's × or a rebuild loses nothing. It writes one note at a time, the newest
  text winning, 500 ms after the last change, and everything at once from `Shell::flush`
  (state saves and quit). A write error is a toast; the text stays and is tried again
  at the next change or flush.
- Edits made to a note's `.md` outside devdeck: read when the note is first loaded; while
  devdeck holds unsaved text, devdeck's wins (notes are devdeck's files).
- `core::view_addr`: `Note { id }` (`note/<id>`); title: "app · name", live through step 3.
  A note deleted while open closes its tabs; a layout naming a deleted note shows a
  message view instead.
- App view: a **Notes** section (alphabetical rows by name), "New note" (button and key
  `n`, asks the name), open on click or ⏎, right-click: rename, delete (confirmed). Notes
  rows join the view's ↑↓ selection with the commands, ⏎ opening a note. An app without
  notes shows only "New note".
- Actions (R5.1): new, open, rename, delete note; toggle source / rendered from step 4.
- Stress and leak cycle (`stress.rs`, R11.4): open and close a file view and a note.
  Numbers in `docs/perf/`.
- Tests (`crates/devdeck-app/tests/notes.rs`): make, type, wait, file on disk; close the tab
  before the write and reopen: the text is there and one write lands; rename keeps the tab
  and the file; delete removes both and closes the tab; a flush saves unsaved text; a
  write error keeps the text and toasts; notes of another app and another workspace stay
  apart; keys typed in a note don't trigger app keys, ctrl+w still closes the tab; ↑↓
  reaches notes rows.

## Visual check (snapshots, and by hand)

File: short, long lines, 10 000 lines, unknown extension, too big, binary, missing,
Markdown source and rendered, narrow pane (title truncates, buttons give way before ×),
150 %, detached window, collapsed. Note: empty, long, rendered with a task list, the list
in the app view with long names, renaming, the delete confirmation, focused. By hand: a
drop from a file manager on Wayland and on X11.

## Out of scope

File explorer; editing files other than notes; HTML; images; notes not tied to an app;
notes in the sidebar or the palette (the first two came later: the sidebar's Notes list,
2026-11-09, R12.3); LSP, completion, go to definition; the Claude control (MCP), next.

## Step 1 spike results (2026-11-08)

Done in a scratch binary rendering offscreen, then a temporary hook in the release
binary; both removed. Outcome: **go on**, with the limits below.

- Builds: gpui-component 0.9.0 resolves with our single `gpui-pre 0.3.11`. The JSON
  grammar comes with `tree-sitter` itself (no `tree-sitter-json` feature).
- Renders without gpui-component's `Root`. `gpui_component::init` is needed (the input's
  own init is private to the crate); its only global key is the inspector's
  `ctrl-shift-i`, debug builds only; every other binding sits in its components' own
  contexts (`Input`, `Dock`, `Table`…), which our views never enter but the editor's.
- Theme: `Theme::change(Dark)` once, then `Theme::update` with our tokens (background,
  foreground, border, muted, selection, caret, link, popover) and our font as both
  `font_family` and `mono_font_family`; the highlight theme is the default dark one with
  our editor background, foreground and line number colors. The Markdown view takes its
  font from the element around it: that element sets our font and size (R1.6).
- Read-only editor: `Editor::readonly(true)`; `soft_wrap(false)` for code; `folding(false)`
  to drop the fold chevrons.
- Scroll: the editor adds both axes of every event (`InputBaseState::on_scroll_wheel`),
  so a sideways swipe drifts (R9.6). `scroll_offset` and `set_scroll_offset` are public:
  our wrapper takes the wheel in the capture phase, locks the axis like the terminal does
  (`gpui::OngoingScroll`), sets the offset and stops the event.
- Key context: `Input` (with `mode=editor`). Its Linux bindings don't include
  `ctrl-shift-e` (macOS only), so the toggle key is free there.
- Cost, release build (thin LTO), idle CPU 0 in every case:

  | | binary | RSS |
  |---|---|---|
  | before | 44.5 MB | 115 MB |
  | gpui-component, Rust grammar only | 56.3 MB | |
  | gpui-component, the 17 grammars | 65.3 MB | 115 MB, no editor shown |
  | 10 000 lines of Rust (430 KB) | | 149 MB |
  | 1 MB of Rust | | 174 MB |
  | 10 MB of Rust, highlighted | | 567 MB |
  | 10 MB, plain text | | 163 MB |
  | one 2 MB minified line, highlighted | | 370 MB |
  | the same, plain text | | 185 MB |
  | DESIGN.md rendered | | 139 MB |

  Highlighting is most of the memory, so: files open up to 10 MB; only files up to 1 MB
  whose lines are all under 5 000 characters are highlighted, others show as plain text
  and the header says so. The binary grows by 21 MB (12 MB the library, 9 MB the
  grammars); the grammar list can shrink later if that matters.

## Plan review (subagent, 2026-11-08) and assessment

1. `ctrl+shift+m` is `pane.maximize`. Taken: `ctrl+shift+e`, checked in the spike.
2. Adding the editor to the `!TextInput` guard turns off pane keys in notes; Esc has no way
   back. Taken: own context, only the keys it takes go off (like terminals), the read-only
   view outside the guard, Esc back to the pane.
3. `gpui_component::init` binds foreign keys, a debug inspector key and sets Light; our
   theme changes run in `app_settings::apply_theme`. Taken: smallest init found in the
   spike, `Theme::update` only, the copy called from `apply_theme`.
4. Its icons aren't served by our `Assets`; its menu and find bar have its own look.
   Taken: icon resolver, our context menu, the find bar checked in the spike.
5. `views::title` takes only the address, so a note's name can't show or update. Taken:
   `title` takes `&App`, in step 3 (an engine change, said so).
6. Header buttons as a closure in `Built` duplicate the registry, act on the focused view,
   and don't redraw. Taken: views publish actions read by header, tab menu and palette;
   clicks target that pane's view; the deck notifies.
7. Notes text owned by views is lost or races on close and quit. Taken: a `Notes` entity
   owning text, one write at a time, flushed from `Shell::flush`; crash order, deleted note
   in a layout, same names, outside edits covered.
8. Notes aren't state (R10.1). Taken: `$XDG_DATA_HOME/devdeck`, to confirm with the user.
9. A direct watch is dropped by the watcher's next rescan; a growing file reparses too often.
   Taken: the `wanted` set with its own route, reload only while shown, at most once a
   second, folder removal handled.
10. `open_at` moves an open view, unlike "focused where it is"; several files at an edge;
    mixes and non-UTF-8 paths; no workspace. Taken: it moves (to confirm), one split for
    several files, the mixes toasted, no workspace toasted.
11. External drops on Wayland and X11. Taken: tests through `FileDropEvent`, a manual drop
    on both. A very fast drop lost on Wayland is GPUI's; noted, not handled.
12. Leak cycle, bigger spike files, requirements first, engine in its own step. Taken all
    four: step 1 commits the docs, step 3 is the engine.
13. Smaller points. Taken: no `default-features` line; dependency weight measured in the
    spike; "edited 3h" dropped (it would go stale); the notes list's exception to R4.2
    written down; notes rows in the app view's selection.

## Build notes (2026-11-08)

- Steps 3 and 4 landed in one commit: the engine pieces (live titles, view actions) have
  their first user and their tests with the file view, and external drops open file views.
- gpui-component's editor needs a window to be made; devdeck builds views without one, so
  a view makes its editor on its first draw. A view moved to another window keeps the
  editor made in the first one: to check by hand (detach a file tab).
- The editor's own keys add `ctrl-h` and `ctrl-enter` to the ones listed by the review.
- Reload timing uses the executor's clock, so tests drive it.
- A view focused before its editor exists (a note just made, a pane focused) hands focus
  on at its next draw; an `on_focus` hook made with the editor came too late.
- `pty_driver::the_environment_is_exactly_the_runs` timed out once (5 s of real time
  waiting for a real process) during a full parallel run, then passed alone three times
  and in the next full run: load-sensitive, not touched by this work.

## Diff review (subagent, 2026-11-08) and assessment

1. A note that can't be read (not UTF-8, no permission) opened empty and the next keystroke
   wrote over it. Taken: `Notes` keeps it as unreadable, the view says so and has no
   editor, nothing is written.
2. The editor's cursor blinked forever, so idle CPU wasn't zero (R1.3). Taken: a steady
   cursor (`CursorBlink::steady`), as our own inputs.
3. Wheel and right-click were caught under a menu, the palette or a modal. Taken: a
   hitbox, `should_handle_scroll` / `is_hovered`.
4. A tab-menu action on a tab behind did nothing (dispatched against a frame without it).
   Taken: `Live::run`, the view runs it itself.
5. Keys: the editor took alt-shift-arrows (moving tabs) on Linux; `EDITOR_KEYS` listed
   macOS-only keys; `?` in the find field opened the cheat sheet. Taken: alt-shift-arrows
   given back to devdeck; the list checked both ways against the keymap; the rule now
   applies to gpui-component's `Input` context (both editors and the find field). Found
   while fixing: the rule matched "Input" inside "TextInput"; context names now compare
   whole. `ctrl-h` stays (bound on Linux).
6. A note detached to another window keeps focus hooks of the first: left for the manual
   check (as noted in the build notes).
7. A Markdown file shown rendered still highlighted its source. Taken: the editor is made
   only when the source shows.
8. Every keystroke in a note redrew the window (two observers, deck notified always).
   Taken: the deck redraws only when a view's title or actions change; one observer.
9. A closed file's folder stayed watched until some change; every log line re-ran the
   watcher's follow. Taken: released with the last view; `Moved` only when the folder to
   watch changes.
10. Smaller: the problem message in a narrow pane (taken: it truncates); the stress note
    (rejected: the stress run is the demo, whose notes stay in memory); `docs/perf`
    numbers (not done in this step: for the milestone's measurements); light themes get
    the dark highlight colors (noted: one theme ships).
