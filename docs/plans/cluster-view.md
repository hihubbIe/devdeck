# Cluster, a full view

Step 4 of "Pulses and full views" (`docs/ROADMAP.md`), after Apps (`apps-view.md`). Agreed
on the mockup (`docs/mockups/views.html`, Cluster, v1); built with `devdeck-ui`
components and theme tokens (R9.3), as devdeck looks, and by the rules the user set on the
way (memory: one page, no filler, grouped actions, fixed columns, no shortcuts outside the
help). Three commits: 4a, 4b, 4c; the user's manual test after 4c.

Requirements: R4.10, R5.1, R5.6, R7.1–R7.8, R9.3, R9.10.

## 4a: two components, and deploying through the picker

- `devdeck_ui::picker`: a modal to pick one of many. Opens with its search focused,
  filters as you type (`devdeck_core::fuzzy`), groups with their headings, optional filter
  chips (all + kinds), rows with a label, chips and a detail; up/down/enter, Esc closes and
  gives the focus back. The caller gives the options and what a pick does; it may offer
  what was typed as its own option. Keys bound in the registry (`picker.*`), context
  `Picker`, listed in the help like every key.
- `devdeck_ui::table`: a header and rows on fixed columns (a width each, the first takes
  the rest, right-aligned where said); a row can hold more under it (opened in place).
  Every cell truncates, nothing shifts with what's empty.
- `pods_bar` without its count (the squares say it; its tooltip says "2 of 3 ready").
- The service view's deploy: the tag editor and its suggestions menu give way to
  **Deploy…**, opening the picker: tags on the other envs (with their env), the branches
  of its app's repository on the remote (age, author), the tags it ran here before, and
  what's typed. A pick goes on to today's checks and confirmation (`save`, R5.6).

## 4b: Cluster as a full view, the service as one page

- `Activity::full_view` takes Cluster: its list, then the selected service's page
  (`svc/<list env>/<name>`), as Apps. A row: its state's dot (running, rolling out,
  failing, stopped), the name, its pods as squares in a slot at the right edge.
- The matrix: an icon in the list's header shows it the whole width (the list hidden);
  its cells open a service as today, over it.
- The service view, one page of panels (no navigation for its own parts; the header
  draws its view actions: its app in Apps, open in pulse):
  - **Deployment**: fields (state, tag, pods, deployed when and by whom, revision; the
    protected env), the rollout under way, then its actions together: Deploy…, Redeploy,
    Notify, Unlock/Lock on a protected env.
  - **Pods**: a table (pod, state, age, restarts, cpu, memory); a row opens in place: node,
    started, last termination, its containers' states (init ones included).
  - **Logs**: the service's logs view, in a panel (the same `LogsView`, reading while the
    page shows).
  - **Envs**: its tag and pods on every env; a row shows that env.
  - **History** and **Containers** (image, ports, requests and limits, configmaps) as
    tables.
- Apps' "go there" to a deployment lands on this page (it's a full view now: no tab).

## 4c: front-end apps inside frontendapp

Read on dev33 (kubectl get, 2026-11-11): `frontendapp` runs one nginx container and one
init container per front-end app (`init-<app>`, image `<app>:<tag>`), copied into a
volume nginx serves.

- `devdeck-kube` reads a deployment's init containers; `Deployment::init_containers`.
- An app whose image is an init container's is that deployment's part: its Apps
  deployments show "frontendapp › monitorapp" with its own tag (`links`).
- The service page gets **Apps**: each part (app, tag), a row's Deploy… deploying that part
  only: the write targets its init container (`initContainers/<i>/image`), its
  confirmation says so.
- Its pods' rows list the init containers and when each ended.

## Not done (later, said so in the roadmap)

- Port-forward and a shell in a pod (`pods/portforward`, `pods/exec` are allowed): their
  own step; no button until they work.
- Each app's public URL and config file (the `frontendapp` configmap): needs the configmap
  read and a configured rule (R1.5).
- Scale, events (not readable), micro-frontends served from the CDN (`aboutapp`,
  `entities-graph`).

## Tests

- 4a: the picker in `tests/ui/` (focus on open, filter, chips, keys, pick, Esc, the typed
  option); the table (fixed columns line up, empty cells keep them, a row opened in
  place); deploying through the picker reaches the confirmation (app tests).
- 4b: Cluster as a full view (list, the page follows the selection, env rows, pivots
  from Apps land there); the page's panels; the matrix mode; the leak cycle.
- 4c: init containers read (kube convert tests); the part linked to its app; the part's
  deploy patch targets its init container; the panel.
- One snapshot per step. 4a and 4c touch `devdeck-core`: the Mac clippy check.
