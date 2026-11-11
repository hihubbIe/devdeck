# One header (R9.11)

The user, 2026-11-11, on a run view in the Agents view showing two headers ("Claude
Code", then "terminal · entities-graph · running ▷ □ ↻ follow"): one header per view,
not "put everything" in it, and its actions fold into "more" when the room runs out.

## Where it was broken

- The run view (commands, terminals, agent sessions): its own row with a state mark, its
  title (an agent's: its terminal's name), the command line, arguments, the state, run,
  stop, restart, follow.
- The branch matrix: its own row "branch matrix" and a usage hint.
- Already right: notes, files, the app, service, port pages, the agents overview.

## What changes

- `Live::status` (`ViewStatus`, `Tone`); the deck keeps it with the title and actions
  (`Deck::view_status`) and redraws when it changes.
- `Toolbar::embedded` and `Toolbar::embedded_width` in `devdeck-ui`.
- The pane header, the tab strip and the details header draw the state and actions
  through one function (`window_view::view_tools`), as an embedded toolbar.
- The run view is `Live`: title (an agent session's name), state, actions: run or stop,
  restart (not for an agent session: it resumes by itself), arguments (a command's),
  follow. Its own row only while arguments are typed. An agent session's header follows
  its state.
- The matrix is `Live`: "branch matrix", "highlighting <tag>"; the hint is a tooltip.

## Tests

The embedded toolbar on its own: icons only however wide, inside the header, folding
into more the last first, as wide as `embedded_width` says. A run's header state and
actions as it starts; the arguments action and its row; an agent session's header (its
name, state and facts, few actions, no row of its own). Header tests moved to the
toolbar's selectors (`tool-<id>`).

Snapshot: none; the user checks it by hand.
