# Ports, a full view

Step 5 of "Pulses and full views" (`docs/ROADMAP.md`), after Terminals and Notes. Agreed on
the mockup (`docs/mockups/views.html`, Ports, 2026-11-11: "GO build it"); built with
`devdeck-ui` components and theme tokens (R9.3), by the rules the user set on the way (one
page, no filler, grouped actions, fixed columns, no shortcuts outside the help). Two
commits, 5a and 5b; the user's manual test after 5b.

Requirements: R4.10, R5.1, R5.6, R6.9, R9.3, R11.6.

## 5a: what the list knows

- `devdeck-sys`:
  - `ports::Listening` gains the address a socket is bound to (`127.0.0.1`, `0.0.0.0`).
  - `ports::all()`: every listening socket of the machine with its holder (a process, or
    one that can't be read). A whole `/proc` walk: only while asked for (the Ports list's
    "machine" toggle on and shown). macOS: unsupported, said so.
  - `procs::started(pid)`: when a process started, in epoch seconds (Linux: boot time
    plus its start ticks). Unsupported elsewhere.
- `devdeck-core::ports::port_forward`: a `kubectl port-forward` command line read
  (namespace, target, local and remote port), for a forward's panel.
- `port_scan`: the scan keeps each port's address; with the machine asked for, everything
  listening (its holder, resolved to a run when it's one's). Terminals' ports count (a
  dev server started in a devdeck terminal was missing).
- `Services::port_rows(machine)`: the Ports list. A port listened on by a run, a terminal
  or a process the workspace knows (declared, or the catalog's), or held by anything when
  "machine" is on; and a known port nobody holds ("free": a command declares it, or one
  was seen on it, from the port memory). Each with who wants it: the commands declaring
  it or seen on it.
- The sidebar's Ports list from it: the state's dot (listening, free), `:port`, the holder
  (or for a free one, the command that wants it), "not devdeck's" for another process.
  Header: a toggle, "machine" (kept in the sidebar's state, off by default). The scan runs
  while the list shows.

## 5b: the page

- `Activity::full_view` takes Ports: its list, then the selected port's page (the port
  view, also a tab).
- One page of panels:
  - **Listening**: fields (state, held by, pid, up since, address, HTTP: a `HEAD /` while
    the page shows, every few seconds, off the UI thread), the command line, then its
    actions together: Open (the browser), Copy URL, Restart (a run), Stop holder… (a run
    at once; another process after its confirmation, as today).
  - **Output**: a run's or terminal's output, the same view a tab shows, at a fixed
    height.
  - **Wants it**: the commands declaring the port or seen on it: how (declared, seen),
    their state, "port held" when another holder would refuse their start; Run, and
    their app in Apps.
  - **Forward**: a `kubectl port-forward`'s env, target, remote and local port; its
    service in Cluster.
  - **Not devdeck's**: another process's, said once.
- The header's icons: its app in Apps, its terminal in Terminals, open in pulse.

## Not done (said so in the roadmap)

- Starting a port-forward from Cluster (its own step, with the shell).
- UDP, other network namespaces (containers' own).

## Tests

- 5a: `ports::all` against a real socket (conformance), the address read, `started`;
  `port_forward` parsing; the rows (a terminal's port, free known ports, wants, machine
  on/off) in app tests with a fake scan.
- 5b: Ports as a full view (list, page follows, the panels by kind, the actions: copy,
  stop, restart; the probe's line), the toggle; one snapshot per step.
- Both touch `devdeck-sys`/`devdeck-core`: the Mac clippy check.
