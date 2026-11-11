//! Renders devdeck states offscreen with the real GPU renderer and text system, and
//! writes one PNG per window per state. Used for the visual check step of every change
//! (docs/WORKFLOW.md). Settings are isolated in a temporary config folder.
//!
//! `cargo run --bin devdeck-snapshot --features devdeck-app/snapshot -- <out-dir> [filter]`
//! where `filter` is a comma-separated list of name fragments (`50-,51-`).

use devdeck_app::cluster::{Cluster, ClusterDriver, EnvUpdate, LogFrom, LogLine, LogRefusal, LogStream, LogUpdate, PodScope, PodUpdate};
use devdeck_app::shell::{Shell, WorkspaceSpec};
use devdeck_app::runner::RunKey;
use devdeck_app::sidebar::{Activity, Sidebar};
use devdeck_app::views::app_view::AppView;
use devdeck_app::views::run_view::RunView;
use devdeck_app::cluster_notes::Notify;
use devdeck_app::views::service_view::ServiceView;
use devdeck_core::view_addr::ViewAddr;
use devdeck_app::{app_settings, gallery::Gallery, window_view::workspace_menu_items};
use devdeck_core::layout::{Drop, Edge, Layout, ViewRef};
use devdeck_ui::{NewToast, ToastKind, logical, open_menu, toasts};
use gpui::{AnyWindowHandle, Entity, Focusable, HeadlessAppContext, point};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

type Extra = fn(&mut HeadlessAppContext, AnyWindowHandle, &Entity<Shell>);

struct State {
    name: &'static str,
    /// The `work` workspace's layout.
    layout: fn() -> Layout,
    settle: Duration,
    scale: f32,
    settings_open: bool,
    extra: Option<Extra>,
    /// Adds the `personal` workspace and shows this one.
    workspaces: Option<&'static str>,
}

fn gallery_layout() -> Layout {
    let mut l = devdeck_app::initial_layout();
    let p = l.main().focused;
    l.open(ViewRef::new("demo/gallery"), p).unwrap();
    l.toggle_maximize(p).unwrap();
    l
}

fn gallery_menu_and_toasts(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let g = cx.update(|cx| deck.read(cx).view(&ViewRef::new("demo/gallery")).unwrap().downcast::<Gallery>().unwrap());
    let _ = cx.update_window(w, |_, window, cx| {
        let filled = g.read(cx).filled.clone();
        filled.read(cx).focus_handle(cx).focus(window, cx);
        for (kind, text) in [
            (ToastKind::Info, "Pulse: Geo work."),
            (ToastKind::Ok, "dev35 › entityservice now runs lan-160."),
            (ToastKind::Warn, "Port 5800 is held by node (pid 7712)."),
            (ToastKind::Err, "boardservice · start:watch exited 1."),
        ] {
            toasts(cx).update(cx, |t, cx| t.push(kind, text, vec![], cx));
        }
        open_menu(point(logical(520.), logical(330.)), devdeck_app::gallery::demo_menu_items(), window, cx);
    });
}

fn gallery_modal(cx: &mut HeadlessAppContext, w: AnyWindowHandle, _shell: &Entity<Shell>) {
    let _ = cx.update_window(w, |_, window, cx| {
        devdeck_app::gallery::open_demo_modal(window, cx);
    });
}

fn workspace_menu(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let _ = cx.update_window(w, |_, window, cx| {
        let items = workspace_menu_items(shell, cx);
        open_menu(point(logical(110.), logical(30.)), items, window, cx);
    });
}

fn quit_guard(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    cx.update(|cx| shell.update(cx, |s, cx| s.request_quit(cx)));
}

fn sidebar(cx: &mut HeadlessAppContext, shell: &Entity<Shell>) -> Entity<Sidebar> {
    cx.update(|cx| shell.read(cx).active_sidebar())
}

/// Pins two apps and selects one, with the list focused.
fn apps_pinned(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let s = sidebar(cx, shell);
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.toggle_pin("entityservice", cx);
            s.toggle_pin("proxyapi", cx);
            s.move_selection(1, cx);
            s.move_selection(1, cx);
            s.move_selection(1, cx);
        })
    });
    let _ = cx.update_window(w, |_, window, cx| s.read(cx).list_focus.clone().focus(window, cx));
}

fn filter(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>, text: &str) {
    let s = sidebar(cx, shell);
    let input = cx.update(|cx| s.read(cx).filter.clone());
    cx.update(|cx| input.update(cx, |i, cx| i.set_text(text, cx)));
    let _ = cx.update_window(w, |_, window, cx| input.read(cx).focus_handle(cx).focus(window, cx));
}

fn filter_matches(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    apps_pinned(cx, w, shell);
    filter(cx, w, shell, "serv");
}

fn filter_none(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    filter(cx, w, shell, "zzz");
}

fn running_only(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let s = sidebar(cx, shell);
    cx.update(|cx| s.update(cx, |s, cx| s.toggle_running_only(cx)));
}

/// A monorepo in `work`'s apps: `mono` holding two apps and two libraries, one running,
/// one that can't run (its tool is missing); `mono` open, `mono/packages` closed.
fn apps_tree(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let services = cx.update(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone());
    let mut catalog = cx.update(|cx| (*services.read(cx).catalog).clone());
    let base = catalog.app("eventservice").unwrap().clone();
    let blocked = catalog.app("remotedesktopapp").unwrap().clone();
    for (id, name, library) in [("mono", "mono", false), ("mono/apps/shell", "shell", false), ("mono/apps/landing", "landing", false)] {
        catalog.apps.push(devdeck_core::apps::App { id: id.into(), name: name.into(), library, ..base.clone() });
    }
    catalog.apps.push(devdeck_core::apps::App { id: "mono/packages/ui".into(), name: "ui".into(), library: true, ..base.clone() });
    catalog.apps.push(devdeck_core::apps::App { id: "mono/packages/infra".into(), name: "infra".into(), ..blocked });
    cx.update(|cx| services.update(cx, |s, cx| s.set_catalog(catalog, cx)));
    run_cmd(cx, shell, "mono/apps/shell", "go:run", 300);
    let s = sidebar(cx, shell);
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.toggle_open("mono", cx);
            s.toggle_open("mono/apps", cx);
            s.selected = Some("mono/packages/".into());
        })
    });
    let _ = cx.update_window(w, |_, window, cx| s.read(cx).list_focus.clone().focus(window, cx));
}

fn apps_tree_filtered(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    apps_tree(cx, w, shell);
    filter(cx, w, shell, "infra");
}

/// Groups in the Apps list: `front-end` holding the monorepo and a group inside, open;
/// `back-end` closed holding two services.
fn make_groups(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    apps_tree(cx, w, shell);
    let s = sidebar(cx, shell);
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.create_group(s.activity(), None, "front-end", cx).unwrap();
            s.create_group(s.activity(), Some("front-end"), "design", cx).unwrap();
            s.create_group(s.activity(), None, "back-end", cx).unwrap();
            s.put(s.activity(), "mono", Some("front-end"), cx);
            s.put(s.activity(), "mono/packages", Some("front-end/design"), cx);
            s.put(s.activity(), "eventservice", Some("back-end"), cx);
            s.put(s.activity(), "graphservice", Some("back-end"), cx);
            s.toggle_open(&devdeck_core::list::group_row_path("back-end"), cx);
            s.selected = Some("mono".into());
        })
    });
}

/// The groups and the right-click menu of an item in one.
fn apps_groups(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    make_groups(cx, w, shell);
    let s = sidebar(cx, shell);
    let _ = cx.update_window(w, |_, window, cx| {
        let items = Sidebar::item_menu(&s, "mono", false, cx);
        open_menu(point(logical(220.), logical(160.)), items, window, cx);
    });
}

/// Pinned folded, All open with groups, then All folded: only the headers.
fn sections_folded(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    apps_pinned(cx, w, shell);
    let s = sidebar(cx, shell);
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.create_group(s.activity(), None, "back-end", cx).unwrap();
            s.toggle_section(devdeck_core::list::Section::Pinned, cx);
            s.toggle_section(devdeck_core::list::Section::All, cx);
        })
    });
}

/// The sidebar dragged wider, with groups.
fn sidebar_wide(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    cx.update(|cx| shell.update(cx, |s, _| s.sidebar_w = Some(520.)));
    make_groups(cx, w, shell);
}

fn cluster_env_menu(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let s = sidebar(cx, shell);
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Cluster, cx)));
    let _ = cx.update_window(w, |_, window, cx| {
        let items = Sidebar::env_menu(&s, cx);
        open_menu(point(logical(190.), logical(34.)), items, window, cx);
    });
}

fn cluster_protected(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let s = sidebar(cx, shell);
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.show(Activity::Cluster, cx);
            s.set_env("hades", cx).expect("the demo has hades");
        })
    });
}

/// The Cluster list on its own env (a protected one) while the workspace keeps dev35,
/// and its env menu open.
fn cluster_list_env(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let s = sidebar(cx, shell);
    cx.update(|cx| {
        s.update(cx, |s, cx| {
            s.show(Activity::Cluster, cx);
            s.set_list_env(Some("hades"), cx);
        })
    });
    cx.run_until_parked();
    let _ = cx.update_window(w, |_, window, cx| {
        let items = Sidebar::list_env_menu(&s, cx);
        open_menu(point(logical(160.), logical(80.)), items, window, cx);
    });
}

fn sidebar_hidden(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let s = sidebar(cx, shell);
    cx.update(|cx| s.update(cx, |s, cx| s.set_visible(false, cx)));
}

fn row_menu(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    apps_pinned(cx, w, shell);
    let s = sidebar(cx, shell);
    let _ = cx.update_window(w, |_, window, cx| {
        let items = Sidebar::item_menu(&s, "boardservice", false, cx);
        open_menu(point(logical(160.), logical(200.)), items, window, cx);
    });
}

fn ports(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let s = sidebar(cx, shell);
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Ports, cx)));
}

fn open_view(cx: &mut HeadlessAppContext, shell: &Entity<Shell>, addr: ViewAddr) {
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let p = l.main().focused;
                l.open(addr.to_ref(), p).map(|_| ())
            });
        })
    });
    cx.run_until_parked();
}

/// A file of this repository, to show in a file view (R12.1).
fn repo_file(rel: &str) -> ViewAddr {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel);
    let path = path.canonicalize().unwrap_or(path);
    ViewAddr::File { path: path.to_string_lossy().into_owned() }
}

fn file_rust(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, repo_file("crates/devdeck-core/src/view_addr.rs"));
}

fn file_md_rendered(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, repo_file("docs/ROADMAP.md"));
}

fn file_md_source(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let addr = repo_file("docs/ROADMAP.md");
    open_view(cx, shell, addr.clone());
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let v = cx.update(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<devdeck_app::views::file_view::FileView>().unwrap());
    let _ = cx.update_window(w, |_, window, cx| v.update(cx, |v, cx| v.toggle_rendered(window, cx)));
}

fn file_not_text(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, repo_file("assets/fonts/jetbrains-mono/JetBrainsMonoNL-Regular.ttf"));
}

fn file_missing(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, repo_file("docs/no-such-file.md"));
}

/// Notes of entityservice (kept in memory: the demo), its app view open; the ids.
fn make_notes(cx: &mut HeadlessAppContext, shell: &Entity<Shell>) -> Vec<String> {
    let notes = cx.update(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).notes.clone());
    let ids = cx.update(|cx| {
        notes.update(cx, |n, cx| {
            let a = n.create("entityservice", "todo", cx).unwrap();
            let b = n.create("entityservice", "a very long note name that the row has to truncate somewhere", cx).unwrap();
            let c = n.create("entityservice", "bugs", cx).unwrap();
            n.set_text(&a, "# Today\n\n- [x] read the graph code\n- [ ] fix the zoom\n\n```rust\nfn zoom(f: f32) -> f32 { f * 2. }\n```\n".into(), cx);
            vec![a, b, c]
        })
    });
    open_view(cx, shell, ViewAddr::App { app: "entityservice".into() });
    ids
}

fn notes_list(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    make_notes(cx, shell);
}

fn note_rendered(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let ids = make_notes(cx, shell);
    open_view(cx, shell, ViewAddr::Note { id: ids[0].clone() });
}

fn note_source(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let ids = make_notes(cx, shell);
    open_view(cx, shell, ViewAddr::Note { id: ids[0].clone() });
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let v = cx.update(|cx| deck.read(cx).view(&ViewAddr::Note { id: ids[0].clone() }.to_ref()).unwrap().downcast::<devdeck_app::views::note_view::NoteView>().unwrap());
    let _ = cx.update_window(w, |_, window, cx| v.update(cx, |v, cx| v.toggle_rendered(window, cx)));
}

fn note_delete(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let ids = make_notes(cx, shell);
    let notes = cx.update(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).notes.clone());
    let _ = cx.update_window(w, |_, window, cx| devdeck_app::views::note_view::delete_note(&notes, &ids[0], window, cx));
}

fn run_cmd(cx: &mut HeadlessAppContext, shell: &Entity<Shell>, app: &str, cmd: &str, wait_ms: u64) {
    let runner = cx.update(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone());
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&RunKey::new(app, cmd), Some("dev35".into()), cx))).expect("starts");
    cx.advance_clock(Duration::from_millis(wait_ms));
    cx.run_until_parked();
}

fn app_view(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    run_cmd(cx, shell, "pagerendererservice", "go:test", 1200);
    run_cmd(cx, shell, "pagerendererservice", "go:run", 1000);
    open_view(cx, shell, ViewAddr::App { app: "pagerendererservice".into() });
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let v = cx.update(|cx| deck.read(cx).view(&ViewAddr::App { app: "pagerendererservice".into() }.to_ref()).unwrap().downcast::<AppView>().unwrap());
    cx.update(|cx| {
        v.update(cx, |v, cx| {
            v.show_hidden = true;
            v.selected = Some("go:build".into());
            cx.notify();
        })
    });
}

fn app_missing_tool(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, ViewAddr::App { app: "remotedesktopapp".into() });
}

fn run_streaming(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, ViewAddr::Run { app: "entityservice".into(), cmd: "go:run".into() });
    cx.advance_clock(Duration::from_secs(9));
    cx.run_until_parked();
}

/// The terminal focused (its cursor shows) with a selection over three rows.
fn run_terminal(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    run_streaming(cx, w, shell);
    let addr = ViewAddr::Run { app: "entityservice".into(), cmd: "go:run".into() };
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let v = cx.update(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<RunView>().unwrap());
    cx.update_window(w, |_, window, cx| v.update(cx, |v, cx| v.focus_terminal(window, cx))).unwrap();
    cx.update(|cx| v.update(cx, |v, cx| v.select((2, 4), (4, 30), cx)));
}

/// Two terminals: one in this layout's focused pane, one left in another layout.
fn terminals(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.save_layout_as("other", cx).unwrap();
            let here = devdeck_app::places::Place::Pane(d.layout().main().focused);
            d.open_terminal(&home.join("Documents/repos"), here, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.switch_layout(devdeck_core::layout_book::DEFAULT_ID, cx).unwrap();
            let here = devdeck_app::places::Place::Pane(d.layout().main().focused);
            d.open_terminal(&home.join("Documents/private_repos/devdeck"), here, cx);
        })
    });
    cx.advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    let _ = w;
}

fn terminals_list(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    terminals(cx, w, shell);
    let s = sidebar(cx, shell);
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Terminals, cx)));
}

/// mastertadaapp's dev server output with two lines selected and the right-click menu:
/// Paste, Copy and Share with (the demo's shells don't take pastes safely: refused).
fn run_context_menu(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    terminals(cx, w, shell);
    let addr = ViewAddr::Run { app: "front-end/mastertadaapp".into(), cmd: "pnpm:start:dev".into() };
    open_view(cx, shell, addr.clone());
    cx.advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let v = cx.update(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<RunView>().unwrap());
    cx.update(|cx| v.update(cx, |v, cx| v.select((1, 0), (2, 30), cx)));
    cx.run_until_parked();
    let at = cx.update(|cx| v.read(cx).cell_center(2, 30)).unwrap_or(point(logical(600.), logical(300.)));
    let _ = cx.update_window(w, |_, window, cx| v.update(cx, |v, cx| v.context_menu(at, window, cx)));
}

/// mastertadaapp's dev server with shift held: its link underlined.
fn run_links(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let addr = ViewAddr::Run { app: "front-end/mastertadaapp".into(), cmd: "pnpm:start:dev".into() };
    open_view(cx, shell, addr.clone());
    cx.advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let v = cx.update(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<RunView>().unwrap());
    cx.update(|cx| v.update(cx, |v, cx| v.show_links(true, cx)));
}

/// eventservice's run with arguments, running; its view.
fn run_with_args(cx: &mut HeadlessAppContext, shell: &Entity<Shell>) -> Entity<RunView> {
    let addr = ViewAddr::Run { app: "eventservice".into(), cmd: "go:run".into() };
    open_view(cx, shell, addr.clone());
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let v = cx.update(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<RunView>().unwrap());
    cx.update(|cx| v.update(cx, |v, cx| v.use_args("serve --port 3001 --verbose", cx)));
    cx.advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    v
}

fn run_args(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    run_with_args(cx, shell);
}

fn run_args_menu(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let v = run_with_args(cx, shell);
    let _ = cx.update_window(w, |_, window, cx| v.update(cx, |v, cx| v.choose_args(window, cx)));
}

fn run_args_typing(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let v = run_with_args(cx, shell);
    let _ = cx.update_window(w, |_, window, cx| v.update(cx, |v, cx| v.type_args(window, cx)));
}

fn run_exited(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    run_cmd(cx, shell, "eventservice", "go:build", 1000);
    open_view(cx, shell, ViewAddr::Run { app: "eventservice".into(), cmd: "go:build".into() });
}

fn run_crashed(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    run_cmd(cx, shell, "boardservice", "go:run", 4200);
    open_view(cx, shell, ViewAddr::Run { app: "boardservice".into(), cmd: "go:run".into() });
}

fn run_not_started(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, ViewAddr::Run { app: "eventservice".into(), cmd: "go:test".into() });
}

/// Pushes a notification and lets `after` pass (toasts leave, the notification stays).
fn notify(cx: &mut HeadlessAppContext, t: NewToast, after: Duration) -> u64 {
    let id = cx.update(|cx| toasts(cx).update(cx, |s, cx| s.push_new(t, cx)));
    cx.run_until_parked();
    cx.advance_clock(after);
    cx.run_until_parked();
    id
}

fn note(kind: ToastKind, text: &str, source: Option<&str>) -> NewToast {
    NewToast::new(kind, text.to_string()).source(source.map(|s| s.to_string().into()))
}

fn bell_unread(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, _shell: &Entity<Shell>) {
    notify(cx, note(ToastKind::Info, "Pulse: Geo work.", None), Duration::from_secs(60));
    notify(cx, note(ToastKind::Warn, "Port 5800 is held by node (pid 7712).", Some("work")), Duration::from_secs(60));
}

fn bell_errors(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    bell_unread(cx, w, shell);
    notify(cx, note(ToastKind::Err, "work · boardservice · go:run exited 2", Some("work")), Duration::from_secs(60));
    notify(cx, note(ToastKind::Err, "dev35 › graphservice: not deployed: image not found", Some("work")), Duration::from_secs(60));
    notify(cx, note(ToastKind::Err, "personal · blog · build exited 1", Some("personal")), Duration::from_secs(60));
}

/// Three toasts (one titled with an action, one long and cut at three lines) and two
/// waiting behind "+2 more".
fn toasts_long(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, _shell: &Entity<Shell>) {
    let run = ViewAddr::Run { app: "boardservice".into(), cmd: "go:run".into() }.to_ref();
    let open = cx.update(|cx| devdeck_app::notifications::reveal_action("work", &run, cx));
    notify(cx, note(ToastKind::Ok, "rollout finished: 3 of 3 pods ready on lan-160", Some("work")).title("graphservice on dev35").note(open), Duration::ZERO);
    let long = "dev35 › entityservice: not deployed: the cluster refused the change because the image registry/entityservice:feature-a-very-long-branch-name-that-goes-on could not be pulled (manifest unknown), and the previous replica set is still serving every request while the new one waits";
    notify(cx, note(ToastKind::Err, long, Some("work")), Duration::ZERO);
    notify(cx, note(ToastKind::Info, "Pulse: Geo work.", None), Duration::ZERO);
    notify(cx, note(ToastKind::Warn, "Port 5800 is held by node (pid 7712).", Some("work")), Duration::ZERO);
    notify(cx, note(ToastKind::Info, "personal · blog · build succeeded in 4 s", Some("personal")), Duration::ZERO);
}

/// The center: read rows (seen on an earlier visit) under fresh ones, of every kind,
/// wrapped text, actions, sources.
fn center_rows(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let main = cx.update(|cx| shell.read(cx).main_window().expect("main"));
    notify(cx, note(ToastKind::Info, "Pulse: Geo work.", None), Duration::from_secs(3 * 3600));
    notify(cx, note(ToastKind::Ok, "personal · blog · build succeeded in 4 s", Some("personal")), Duration::from_secs(40 * 60));
    let _ = cx.update(|cx| main.update(cx, |w, window, cx| w.open_notifications(None, window, cx)));
    cx.run_until_parked();
    let _ = cx.update(|cx| main.update(cx, |w, window, cx| w.uncover(window, cx)));
    cx.run_until_parked();
    let problems = devdeck_app::notifications::problems_action("View");
    notify(cx, note(ToastKind::Warn, "2 problems reading the config or the projects", None).note(problems), Duration::from_secs(12 * 60));
    let long = "dev35 › entityservice: not deployed: the cluster refused the change because the image registry/entityservice:feature-a-very-long-branch-name-that-goes-on could not be pulled (manifest unknown), and the previous replica set is still serving every request while the new one waits";
    notify(cx, note(ToastKind::Err, long, Some("work")), Duration::from_secs(5 * 60));
    let run = ViewAddr::Run { app: "boardservice".into(), cmd: "go:run".into() }.to_ref();
    let open = cx.update(|cx| devdeck_app::notifications::reveal_action("work", &run, cx));
    notify(cx, note(ToastKind::Ok, "rollout finished: 3 of 3 pods ready on lan-160", Some("work")).title("graphservice on dev35").note(open), Duration::from_secs(2 * 60));
    let _ = cx.update(|cx| main.update(cx, |w, window, cx| w.open_notifications(None, window, cx)));
    cx.run_until_parked();
    // The toasts of this visit gone: the center alone.
    cx.update(|cx| {
        toasts(cx).update(cx, |t, cx| {
            t.queue.items.clear();
            cx.notify();
        })
    });
}

/// Notifications step B: a revealed tab (behind another, in a pane not focused) mid-flash,
/// its tab and its pane's border in the accent, fading.
fn reveal_flash(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let matrix = l.find_view(&devdeck_core::layout::ViewRef::new("matrix")).expect("matrix").0;
                l.focus(matrix)
            })
        })
    });
    cx.update(|cx| shell.update(cx, |s, cx| s.reveal("work", &devdeck_core::layout::ViewRef::new("demo/gallery"), cx))).expect("revealed");
    cx.run_until_parked();
    // Drawn once (the fade starts), then 40 % of the way.
    let _ = cx.update_window(w, |_, window, _| window.refresh());
    cx.run_until_parked();
    cx.advance_clock(devdeck_app::deck::FLASH * 2 / 5);
    cx.run_until_parked();
}

/// The same with reduced motion: the flash drawn still, at full strength.
fn reveal_flash_still(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    cx.update(|cx| cx.set_reduce_motion(true));
    reveal_flash(cx, w, shell);
}

fn work_cluster(cx: &mut HeadlessAppContext, shell: &Entity<Shell>) -> Entity<devdeck_app::cluster::Cluster> {
    cx.update(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).cluster.clone().unwrap())
}

/// Opens a service view, maximized, and waits for its env to load.
fn service(cx: &mut HeadlessAppContext, shell: &Entity<Shell>, env: &str, name: &str, maximize: bool) -> Entity<ServiceView> {
    let addr = ViewAddr::Svc { env: env.into(), name: name.into() };
    open_view(cx, shell, addr.clone());
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    if maximize {
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.toggle_maximize(l.main().focused))));
    }
    cx.advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<ServiceView>().unwrap())
}

fn deploy(cx: &mut HeadlessAppContext, shell: &Entity<Shell>, env: &str, name: &str, tag: &str, wait_ms: u64) {
    let c = work_cluster(cx, shell);
    cx.update(|cx| {
        c.update(cx, |c, cx| {
            c.want(env, cx);
        })
    });
    cx.advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let from = cx.update(|cx| c.read(cx).deployment(env, name).map(|d| d.main().image.raw.clone())).expect("deployed");
    cx.update(|cx| c.update(cx, |c, cx| c.deploy(env, name, &from, tag, true, confirmed(), cx))).expect("deploys");
    cx.advance_clock(Duration::from_millis(wait_ms));
    cx.run_until_parked();
}

fn svc_ready(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    deploy(cx, shell, "dev33", "entityservice", "lan-150", 3500);
    service(cx, shell, "dev35", "entityservice", true);
    cx.advance_clock(Duration::from_secs(6));
    cx.run_until_parked();
}

fn svc_rolling(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    service(cx, shell, "dev35", "boardservice", true);
    deploy(cx, shell, "dev35", "boardservice", "lan-160", 1000);
}

fn svc_failed(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    service(cx, shell, "dev35", "graphservice", true);
    deploy(cx, shell, "dev35", "graphservice", "lan-broken", 1100);
}

fn svc_protected(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let v = service(cx, shell, "hades", "proxyapi", true);
    cx.update(|cx| v.update(cx, |v, cx| v.tag.update(cx, |i, cx| i.set_text("lan-161", cx))));
}

fn svc_confirm(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let v = service(cx, shell, "dev35", "entityservice", true);
    cx.update(|cx| v.update(cx, |v, cx| v.tag.update(cx, |i, cx| i.set_text("lan-160", cx))));
    let _ = cx.update_window(w, |_, window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|cx| v.update(cx, |v, cx| v.save(cx)));
}

/// The confirmation after the cluster refused: someone deployed since it opened.
fn svc_confirm_refused(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    svc_confirm(cx, w, shell);
    cx.run_until_parked();
    let c = work_cluster(cx, shell);
    cx.update(|cx| {
        c.update(cx, |c, cx| {
            let mut list: Vec<_> = c.peek("dev35").ready().map(|m| m.values().cloned().collect()).unwrap_or_default();
            if let Some(d) = list.iter_mut().find(|d| d.name == "entityservice") {
                // The whole image: the fake's guard compares it as written.
                let repo = d.main().image.repo.clone();
                d.main_mut().image = devdeck_core::cluster::Image::parse(&format!("{repo}:hotfix-from-slack"));
                d.revision += 1;
            }
            c.apply("dev35", EnvUpdate::Deployments(list), cx);
        })
    });
    // Tab to Deploy (the modal opens on Cancel), then Enter.
    let _ = cx.update_window(w, |_, window, cx| {
        window.dispatch_action(Box::new(devdeck_ui::overlay::ModalNext), cx);
        window.dispatch_action(Box::new(devdeck_app::views::service_view::ConfirmModal), cx);
    });
    cx.run_until_parked();
}

/// A dangerous action sent without its modal: the snapshots set up states directly.
#[allow(clippy::disallowed_methods)]
fn confirmed() -> devdeck_app::danger::Confirmed {
    devdeck_app::danger::for_tests()
}

/// dev35's first service whose main container doesn't pull `Always` (the demo's
/// `IfNotPresent`).
fn if_not_present() -> String {
    let services = devdeck_core::demo::catalog("work").cluster.expect("cluster").services;
    services.into_iter().find(|s| devdeck_core::demo::deployment("dev35", s).is_some_and(|d| d.main().pull_policy() != "Always")).expect("a service pulling IfNotPresent")
}

/// The redeploy confirmation, with the pull policy's warning.
fn svc_redeploy_confirm(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let v = service(cx, shell, "dev35", &if_not_present(), true);
    let _ = cx.update_window(w, |_, window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|cx| v.update(cx, |v, cx| v.redeploy(cx)));
}

/// A redeploy rolling: one new pod of two on the same tag.
fn svc_redeploying(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    service(cx, shell, "dev35", "boardservice", true);
    let c = work_cluster(cx, shell);
    let image = cx.update(|cx| c.read(cx).deployment("dev35", "boardservice").map(|d| d.main().image.raw.clone())).expect("deployed");
    cx.update(|cx| c.update(cx, |c, cx| c.redeploy("dev35", "boardservice", &image, true, confirmed(), cx))).expect("redeploys");
    cx.advance_clock(Duration::from_millis(1000));
    cx.run_until_parked();
}

fn svc_suggestions(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    deploy(cx, shell, "dev35", "entityservice", "lan-160", 3500);
    let v = service(cx, shell, "dev35", "entityservice", true);
    let items = cx.update(|cx| ServiceView::suggestions_for(&v, cx));
    let _ = cx.update_window(w, |_, window, cx| open_menu(point(logical(720.), logical(140.)), items, window, cx));
}

/// Another tag typed: the rocket ready (in the accent), "unsaved" at the row's end.
fn svc_tag_ready(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let v = service(cx, shell, "dev35", "entityservice", true);
    cx.update(|cx| v.update(cx, |v, cx| v.tag.update(cx, |i, cx| i.set_text("lan-160", cx))));
}

/// The notify dropdown open, two options ticked: the bell in the accent.
fn svc_notify(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let v = service(cx, shell, "dev35", "entityservice", true);
    let c = work_cluster(cx, shell);
    cx.update(|cx| {
        c.update(cx, |c, cx| {
            c.set_notify("dev35", "entityservice", Notify::Finished, true, cx);
            c.set_notify("dev35", "entityservice", Notify::Failed, true, cx);
        })
    });
    let _ = cx.update_window(w, |_, window, _| window.refresh());
    cx.run_until_parked();
    let _ = cx.update_window(w, |_, window, cx| v.update(cx, |v, cx| v.open_notify(window, cx)));
    cx.run_until_parked();
}

fn svc_loading(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, ViewAddr::Svc { env: "dev34".into(), name: "vesselservice".into() });
}

fn svc_unreachable(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    service(cx, shell, "dev36", "vesselservice", false);
}

fn svc_forbidden_narrow(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    service(cx, shell, "lab", "tadawebapi", false);
}

/// dev35's first deployment running more than one container (the demo's proxy).
fn multi_container(cx: &mut HeadlessAppContext, shell: &Entity<Shell>) -> String {
    let c = work_cluster(cx, shell);
    cx.update(|cx| c.update(cx, |c, cx| c.want("dev35", cx)));
    cx.advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update(|cx| c.read(cx).names().into_iter().find(|n| c.read(cx).deployment("dev35", n).is_some_and(|d| d.others() > 0)).map(str::to_string)).expect("a deployment with a proxy")
}

/// M3: dev35's connection lost after a list: the list stays, the problem shows, in the
/// sidebar and the service view (several containers).
fn svc_lost_connection(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let name = multi_container(cx, shell);
    let s = sidebar(cx, shell);
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Cluster, cx)));
    service(cx, shell, "dev35", &name, false);
    let c = work_cluster(cx, shell);
    let problem = EnvUpdate::Problem(Some("can't reach km011gcp (dev35): connection refused".into()));
    cx.update(|cx| c.update(cx, |c, cx| c.apply("dev35", problem, cx)));
}

/// As the kube driver: pods watched for whoever wants them (the demo's), no logs, no
/// deploys.
struct KubeLike;

impl ClusterDriver for KubeLike {
    fn watch(&self, _env: &str, _cx: &mut gpui::Context<Cluster>) -> Option<gpui::Task<()>> {
        None
    }
    fn namespace(&self, env: &str) -> String {
        env.into()
    }
    fn writes(&self) -> Result<(), String> {
        Ok(())
    }
    fn watch_pods(&self, env: &str, scope: PodScope, _selector: Option<&str>, cx: &mut gpui::Context<Cluster>) -> Option<gpui::Task<()>> {
        let env = env.to_string();
        Some(cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |c, cx| {
                let pods = match &scope {
                    PodScope::Service(name) => c.deployment(&env, name).map(|d| devdeck_core::demo::pods(&env, d)).unwrap_or_default(),
                    PodScope::Env => Vec::new(),
                };
                c.apply_pods(&env, scope, PodUpdate::Pods(pods), cx);
            });
        }))
    }
}

/// A service on the kube driver: its pods watched while open (one crash-looping), logs
/// and deploys refused with their reasons, metrics not read yet.
fn svc_kube_like(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let c = work_cluster(cx, shell);
    cx.update(|cx| c.update(cx, |c, cx| c.want("dev35", cx)));
    cx.advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let name = cx.update(|cx| c.read(cx).names().into_iter().find(|n| c.read(cx).deployment("dev35", n).is_some_and(|d| d.ready < d.replicas)).map(str::to_string)).expect("a deployment with a pod down");
    cx.update(|cx| c.update(cx, |c, cx| c.set_driver(c.envs.clone(), std::rc::Rc::new(KubeLike), cx)));
    service(cx, shell, "dev35", &name, true);
}

/// The Cluster list with **watch** on: deployments with a pod in trouble are marked.
fn cluster_watch(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let s = sidebar(cx, shell);
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Cluster, cx)));
    cx.advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update(|cx| s.update(cx, |s, cx| s.toggle_watch_pods(cx)));
}

/// A throwaway repository: an origin with main, feat and a fix branch, a clone on main with
/// a local branch and two changes. Made with `git`, isolated from the machine's config.
fn throwaway_repo() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("devdeck-snapshot-git-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let git = |dir: &std::path::Path, args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "a@x")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "a@x")
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(ok, "git {args:?}");
    };
    git(&base, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
    git(&base, &["clone", "-q", "origin.git", "entityservice"]);
    let repo = base.join("entityservice");
    std::fs::write(repo.join("main.go"), "package main\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "feat: entity search by type"]);
    git(&repo, &["push", "-q", "-u", "origin", "main"]);
    for b in ["ET-1502-oidc-session-cookie-scope", "lan-160"] {
        git(&repo, &["push", "-q", "origin", &format!("main:{b}")]);
    }
    git(&repo, &["fetch", "-q"]);
    git(&repo, &["branch", "fix-search-paging"]);
    std::fs::write(repo.join("main.go"), "package main\n// wip\n").unwrap();
    std::fs::write(repo.join("notes.md"), "todo\n").unwrap();
    repo
}

fn git_panel(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let root = throwaway_repo();
    open_view(cx, shell, ViewAddr::Git { root: root.display().to_string() });
    maximize_focused(cx, shell);
    // Git runs on real threads: wait for the read.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let addr = ViewAddr::Git { root: root.display().to_string() }.to_ref();
    loop {
        cx.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        let read = cx.update(|cx| {
            deck.read(cx).view(&addr).and_then(|v| v.downcast::<devdeck_app::views::git_view::GitView>().ok()).is_some_and(|v| v.read(cx).repo.read(cx).snapshot().is_some())
        });
        if read || std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The git panel with its remote branches shown.
fn git_panel_remotes(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    git_panel(cx, w, shell);
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    cx.update(|cx| {
        let views: Vec<_> = deck.read(cx).layout().views().into_iter().collect();
        for r in views {
            if let Some(v) = deck.read(cx).view(&r).and_then(|v| v.downcast::<devdeck_app::views::git_view::GitView>().ok()) {
                v.update(cx, |v, cx| v.toggle_remotes(cx));
            }
        }
    });
    cx.run_until_parked();
}

/// The git panel, remote branches shown, in a narrow pane: names and messages cut, the
/// author and the age at each row's end.
fn git_panel_narrow(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    git_panel_remotes(cx, w, shell);
    // Back from maximized, and a pane beside it: the panel gets a narrow column.
    maximize_focused(cx, shell);
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let p = l.main().focused;
                l.open(ViewRef::new("demo/c"), p)?;
                let i = l.pane(p)?.tabs.len() - 1;
                l.move_tab(p, i, Drop::Edge(p, Edge::Right)).map(|_| ())
            })
        })
    });
    cx.run_until_parked();
}

fn maximize_focused(cx: &mut HeadlessAppContext, shell: &Entity<Shell>) {
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.toggle_maximize(l.main().focused))));
    cx.run_until_parked();
}

fn logs_find(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let addr = ViewAddr::Logs { env: "dev35".into(), name: "graphservice".into() };
    open_view(cx, shell, addr.clone());
    maximize_focused(cx, shell);
    cx.advance_clock(Duration::from_secs(20));
    cx.run_until_parked();
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let v = cx.update(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<devdeck_app::views::logs_view::ServiceLogsView>().unwrap());
    cx.update(|cx| {
        v.update(cx, |v, cx| {
            v.set_min_level(devdeck_app::views::logs_view::MinLevel::Warn, cx);
            v.find.update(cx, |f, cx| f.set_text("slow", cx));
        })
    });
}

/// As the kube driver, for logs: the demo's pods of a service, the last one waiting on its
/// image; each running pod's stream answers JSON lines (levels in them), or every stream is
/// refused (`forbid`). A read back finds older lines, or never answers (`hang`).
struct LogsLike {
    forbid: bool,
    hang: bool,
}

/// The time of the first line of `pod`'s stream on [`LogsLike`].
fn logs_like_base(pod: &str) -> i64 {
    let seed = devdeck_core::demo::hash(pod) % 1000;
    (devdeck_core::demo::NOW_S as i64 - 120) * 1_000_000_000 + seed as i64 * 1_000_000
}

const LOGS_LIKE_FIRST: &str = r#"{"level":"info","msg":"GET /v1/graph/entities 200","ms":12}"#;

impl ClusterDriver for LogsLike {
    fn watch(&self, _env: &str, _cx: &mut gpui::Context<Cluster>) -> Option<gpui::Task<()>> {
        None
    }
    fn namespace(&self, env: &str) -> String {
        env.into()
    }
    fn writes(&self) -> Result<(), String> {
        Ok(())
    }
    fn watch_pods(&self, env: &str, scope: PodScope, _selector: Option<&str>, cx: &mut gpui::Context<Cluster>) -> Option<gpui::Task<()>> {
        let env = env.to_string();
        Some(cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |c, cx| {
                let mut pods = match &scope {
                    PodScope::Service(name) => c.deployment(&env, name).map(|d| devdeck_core::demo::pods(&env, d)).unwrap_or_default(),
                    PodScope::Env => Vec::new(),
                };
                for p in &mut pods {
                    p.ready = true;
                    p.phase = devdeck_core::cluster::PodPhase::Running;
                    p.containers[0].state = "running".into();
                }
                if let Some(last) = pods.last_mut() {
                    last.containers[0].state = "waiting: ImagePullBackOff".into();
                    last.phase = devdeck_core::cluster::PodPhase::Waiting("ImagePullBackOff".into());
                }
                c.apply_pods(&env, scope, PodUpdate::Pods(pods), cx);
            });
        }))
    }
    fn stream_logs(&self, env: &str, pod: &str, _container: &str, _from: LogFrom, _previous: bool, _cx: &mut gpui::Context<Cluster>) -> Result<LogStream, String> {
        let (tx, updates) = async_channel::unbounded();
        if self.forbid {
            let _ = tx.try_send(LogUpdate::Refused(LogRefusal::Forbidden(format!("your account can't read logs in {env}"))));
        } else {
            let base = logs_like_base(pod);
            let texts = [
                LOGS_LIKE_FIRST.to_string(),
                r#"{"level":"warn","msg":"slow query entities.find","ms":840}"#.to_string(),
                r#"{"level":30,"msg":"rabbitmq connected","host":"rabbitmq-0.rabbitmq"}"#.to_string(),
                r#"{"level":"error","msg":"publish failed: channel closed","exchange":"graph.events","retry":3}"#.to_string(),
                "plain text line from a library that knows no JSON".to_string(),
                r#"{"level":"debug","msg":"cache hit","key":"entity:0a3f9c21"}"#.to_string(),
            ];
            let lines = texts
                .iter()
                .enumerate()
                .map(|(i, t)| LogLine { at: Some(base + i as i64 * 7_000_000_000), level: devdeck_core::runs::log_level(t), text: t.clone() })
                .collect();
            let _ = tx.try_send(LogUpdate::Lines(lines));
        }
        Ok(LogStream { updates, keep: Box::new(tx) })
    }
    fn read_logs(&self, _env: &str, pod: &str, _container: &str, _tail: Option<u32>, _cx: &mut gpui::Context<Cluster>) -> Result<LogStream, String> {
        let (tx, updates) = async_channel::unbounded();
        if !self.hang {
            let base = logs_like_base(pod);
            let older = [r#"{"level":"info","msg":"starting graphservice","version":"main-4f2a"}"#, r#"{"level":"info","msg":"rabbitmq connecting","host":"rabbitmq-0.rabbitmq"}"#, "plain text line from before the first one shown"];
            let mut lines: Vec<LogLine> = older.iter().enumerate().map(|(i, t)| LogLine { at: Some(base - (3 - i as i64) * 5_000_000_000), level: devdeck_core::runs::log_level(t), text: t.to_string() }).collect();
            lines.push(LogLine { at: Some(base), level: devdeck_core::runs::log_level(LOGS_LIKE_FIRST), text: LOGS_LIKE_FIRST.into() });
            let _ = tx.try_send(LogUpdate::Lines(lines));
            let _ = tx.try_send(LogUpdate::Ended);
        }
        Ok(LogStream { updates, keep: Box::new(tx) })
    }
}

/// A service's logs on a kube-like driver: two pods' JSON lines merged by time, levels
/// read from them, a pod waiting on its image; or every stream refused (`forbid`).
fn logs_kube_like(cx: &mut HeadlessAppContext, shell: &Entity<Shell>, forbid: bool) {
    logs_kube_like_reading(cx, shell, forbid, false);
}

/// The same, a read back never answering when `hang`.
fn logs_kube_like_reading(cx: &mut HeadlessAppContext, shell: &Entity<Shell>, forbid: bool, hang: bool) {
    let c = work_cluster(cx, shell);
    cx.update(|cx| c.update(cx, |c, cx| c.want("dev35", cx)));
    cx.advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update(|cx| c.update(cx, |c, cx| c.set_driver(c.envs.clone(), std::rc::Rc::new(LogsLike { forbid, hang }), cx)));
    open_view(cx, shell, ViewAddr::Logs { env: "dev35".into(), name: "graphservice".into() });
    maximize_focused(cx, shell);
    cx.advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
}

/// Splits the focused pane `times`, the logs staying in the left part: a narrower pane each
/// time (the toolbar's compact and narrow forms).
fn narrow_logs(cx: &mut HeadlessAppContext, shell: &Entity<Shell>, times: usize) {
    maximize_focused(cx, shell);
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    for n in 0..times {
        cx.update(|cx| {
            deck.update(cx, |d, cx| {
                d.apply(cx, |l| {
                    let p = l.main().focused;
                    l.open(ViewRef::new(format!("demo/n{n}")), p)?;
                    let i = l.pane(p)?.tabs.len() - 1;
                    l.move_tab(p, i, Drop::Edge(p, Edge::Right)).map(|_| ())
                })
            })
        });
        cx.run_until_parked();
        let logs = ViewAddr::Logs { env: "dev35".into(), name: "graphservice".into() }.to_ref();
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.find_view(&logs).map(|(p, i)| l.activate(p, i)).unwrap_or(Ok(())))));
    }
    cx.run_until_parked();
}

/// The logs filtered to warn+ and paused, in a pane half as wide: compact (icons only, the
/// level keeping its value).
fn logs_medium(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    logs_kube_like(cx, shell, false);
    narrow_logs(cx, shell, 1);
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let addr = ViewAddr::Logs { env: "dev35".into(), name: "graphservice".into() };
    let v = cx.update(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<devdeck_app::views::logs_view::ServiceLogsView>().unwrap());
    cx.update(|cx| v.update(cx, |v, cx| v.set_min_level(devdeck_app::views::logs_view::MinLevel::Warn, cx)));
    cx.run_until_parked();
}

/// The logs in a narrow pane: follow, wrap and pause folded into "more".
fn logs_narrow(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    logs_kube_like(cx, shell, false);
    narrow_logs(cx, shell, 3);
}

fn logs_json(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    logs_kube_like(cx, shell, false);
}

/// Earlier lines read with `logs.earlier`: loading (`hang`), else read to the start.
fn logs_earlier(cx: &mut HeadlessAppContext, shell: &Entity<Shell>, hang: bool) {
    logs_kube_like_reading(cx, shell, false, hang);
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let addr = ViewAddr::Logs { env: "dev35".into(), name: "graphservice".into() };
    let v = cx.update(|cx| deck.read(cx).view(&addr.to_ref()).unwrap().downcast::<devdeck_app::views::logs_view::ServiceLogsView>().unwrap());
    cx.update(|cx| v.update(cx, |v, cx| v.earlier(cx)));
    cx.run_until_parked();
}

fn logs_earlier_loading(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    logs_earlier(cx, shell, true);
}

fn logs_earlier_start(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    logs_earlier(cx, shell, false);
}

fn logs_refused(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    logs_kube_like(cx, shell, true);
}

fn pod(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    let c = work_cluster(cx, shell);
    cx.update(|cx| {
        c.update(cx, |c, cx| {
            c.want("dev35", cx);
        })
    });
    cx.advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let name = cx.update(|cx| {
        let c = c.read(cx);
        c.names().into_iter().flat_map(|s| c.pods("dev35", s).to_vec()).find(|p| !p.ready).map(|p| p.name).unwrap_or_default()
    });
    open_view(cx, shell, ViewAddr::Pod { env: "dev35".into(), pod: name });
}

fn matrix(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, ViewAddr::Matrix);
    maximize_focused(cx, shell);
    cx.advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let m = cx.update(|cx| deck.read(cx).view(&ViewAddr::Matrix.to_ref()).unwrap().downcast::<devdeck_app::views::matrix_view::MatrixView>().unwrap());
    cx.update(|cx| {
        m.update(cx, |m, cx| {
            m.highlight = Some("main".into());
            cx.notify();
        })
    });
}

fn port(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, ViewAddr::Port { port: "5720".into() });
}

fn cheat_sheet(cx: &mut HeadlessAppContext, w: AnyWindowHandle, _shell: &Entity<Shell>) {
    let _ = cx.update_window(w, |_, window, cx| devdeck_app::cheat_sheet::open(window, cx));
    cx.run_until_parked();
}

fn cheat_sheet_filtered(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    cheat_sheet(cx, w, shell);
    cx.update(|cx| {
        let layer = devdeck_ui::overlay::layer(w, cx);
        let modal = layer.read(cx).modals.last().cloned().unwrap();
        let sheet = modal.downcast::<devdeck_app::cheat_sheet::CheatSheet>().unwrap();
        let input = sheet.read(cx).filter.clone();
        input.update(cx, |i, cx| i.set_text("tab", cx));
    });
}

fn palette_with(text: &'static str, drill: bool) -> impl Fn(&mut HeadlessAppContext, AnyWindowHandle) {
    move |cx, w| {
        let _ = cx.update_window(w, |_, window, cx| devdeck_app::palette::open(window, cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let layer = devdeck_ui::overlay::layer(w, cx);
            let modal = layer.read(cx).modals.last().cloned().unwrap();
            let p = modal.downcast::<devdeck_app::palette::Palette>().unwrap();
            let input = p.read(cx).input.clone();
            input.update(cx, |i, cx| i.set_text(text, cx));
            if drill {
                p.update(cx, |p, cx| {
                    p.drill = Some("entityservice".into());
                    cx.notify();
                });
                input.update(cx, |i, cx| i.set_text("", cx));
            }
        });
    }
}

fn palette_empty(cx: &mut HeadlessAppContext, w: AnyWindowHandle, _s: &Entity<Shell>) {
    palette_with("", false)(cx, w);
}
fn palette_matches(cx: &mut HeadlessAppContext, w: AnyWindowHandle, _s: &Entity<Shell>) {
    palette_with("ent", false)(cx, w);
}
fn palette_drill(cx: &mut HeadlessAppContext, w: AnyWindowHandle, _s: &Entity<Shell>) {
    palette_with("", true)(cx, w);
}
fn palette_env(cx: &mut HeadlessAppContext, w: AnyWindowHandle, _s: &Entity<Shell>) {
    palette_with("event run @33", false)(cx, w);
}

fn tab_menu(cx: &mut HeadlessAppContext, w: AnyWindowHandle, _s: &Entity<Shell>) {
    let _ = cx.update_window(w, |_, window, cx| window.dispatch_action(Box::new(devdeck_app::actions::ContextMenu), cx));
}

/// The tab menu's last item, "Open terminal here", with its places open beside it.
fn tab_menu_places(cx: &mut HeadlessAppContext, w: AnyWindowHandle, s: &Entity<Shell>) {
    tab_menu(cx, w, s);
    cx.run_until_parked();
    let _ = cx.update_window(w, |_, window, cx| window.dispatch_action(Box::new(devdeck_ui::overlay::MenuUp), cx));
    cx.run_until_parked();
    let _ = cx.update_window(w, |_, window, cx| window.dispatch_action(Box::new(devdeck_ui::overlay::MenuOpenSub), cx));
}

fn send_to(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let (p, i) = l.find_view(&ViewRef::new("demo/notes")).unwrap();
                l.detach(p, i).map(|_| ())
            });
        })
    });
    cx.run_until_parked();
    let source = cx.update(|cx| {
        let l = deck.read(cx).layout();
        devdeck_app::deck::SendSource::Tab(l.pane(l.main().focused).unwrap().active_view().unwrap().clone())
    });
    let _ = cx.update_window(w, |_, window, cx| devdeck_app::send_to::open(deck.clone(), source, window, cx));
}

fn with_layouts(cx: &mut HeadlessAppContext, shell: &Entity<Shell>) -> Entity<devdeck_app::deck::Deck> {
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.save_layout_as("triage", cx).unwrap();
            d.save_layout_as("incident dev35 long name for truncation", cx).unwrap();
            d.save_layout_as("focus", cx).unwrap();
        })
    });
    deck
}

fn layout_menu(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let deck = with_layouts(cx, shell);
    let _ = cx.update_window(w, |_, window, cx| {
        let items = devdeck_app::layouts::menu(&deck, cx);
        open_menu(point(logical(260.), logical(34.)), items, window, cx);
    });
}

fn layouts_list(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    with_layouts(cx, shell);
    let s = sidebar(cx, shell);
    cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Layouts, cx)));
}

fn layout_save_error(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    let deck = with_layouts(cx, shell);
    let _ = cx.update_window(w, |_, window, cx| devdeck_app::layouts::open_save_as(deck.clone(), window, cx));
    cx.run_until_parked();
    cx.update(|cx| {
        let layer = devdeck_ui::overlay::layer(w, cx);
        let modal = layer.read(cx).modals.last().cloned().unwrap();
        let m = modal.downcast::<devdeck_app::name_prompt::NamePrompt>().unwrap();
        let input = m.read(cx).input.clone();
        input.update(cx, |i, cx| i.set_text("Triage", cx));
        m.update(cx, |m, cx| {
            m.error = Some("a pulse called triage exists".into());
            cx.notify();
        });
    });
}

fn pane_of(l: &Layout, v: &str) -> devdeck_core::layout::PaneId {
    l.find_view(&ViewRef::new(v)).expect("view open").0
}

fn collapsed_column() -> Layout {
    let mut l = devdeck_app::initial_layout();
    let p = pane_of(&l, "demo/notes");
    l.toggle_collapse(p).unwrap();
    l
}

fn collapsed_row() -> Layout {
    let mut l = devdeck_app::initial_layout();
    let p = pane_of(&l, "demo/logs");
    l.toggle_collapse(p).unwrap();
    l
}

/// The focus on a one-tab pane: its header's accent line (R3.12).
fn one_tab_focused() -> Layout {
    let mut l = devdeck_app::initial_layout();
    let p = pane_of(&l, "matrix");
    l.focus(p).unwrap();
    l
}

fn maximized() -> Layout {
    let mut l = devdeck_app::initial_layout();
    let p = pane_of(&l, "matrix");
    l.toggle_maximize(p).unwrap();
    l
}

fn narrow() -> Layout {
    let mut l = devdeck_app::initial_layout();
    let p = pane_of(&l, "demo/logs");
    for v in ["demo/a-very-long-view-name-that-must-truncate", "demo/b-another-long-one-for-truncation", "demo/c"] {
        l.open(ViewRef::new(v), p).unwrap();
        let i = l.pane(p).unwrap().tabs.len() - 1;
        l.move_tab(p, i, Drop::Edge(p, Edge::Right)).unwrap();
    }
    l
}

fn detached() -> Layout {
    let mut l = devdeck_app::initial_layout();
    let (p, i) = l.find_view(&ViewRef::new("demo/notes")).unwrap();
    l.detach(p, i).unwrap();
    l
}

/// What devdeck shows with no workspace configured (main.rs opens it then).
fn welcome() -> Layout {
    let mut l = Layout::new();
    let p = l.main().focused;
    l.open(ViewRef::new("welcome"), p).expect("open the welcome view");
    l
}

/// The control API off (no runtime folder): Settings says why.
fn api_off(cx: &mut HeadlessAppContext, _: AnyWindowHandle, _: &Entity<Shell>) {
    cx.update(|cx| {
        let dir = std::env::temp_dir();
        devdeck_app::api::start(devdeck_app::api::Config { runtime: devdeck_app::api::no_runtime(), state_dir: dir.clone(), default_state_dir: dir, audit_file: false }, cx);
    });
}

/// The control API on, with callers (one with a name long enough to be cut) and audit
/// entries. Shown as it would be; no socket is made.
fn api_callers(cx: &mut HeadlessAppContext, _: AnyWindowHandle, _: &Entity<Shell>) {
    use devdeck_app::api::audit::{Entry, Event};
    use devdeck_app::api::{Connection, Status, Who};
    let api = cx.update(|cx| {
        let dir = PathBuf::from("/home/u/.local/state/devdeck");
        devdeck_app::api::start(devdeck_app::api::Config { runtime: devdeck_app::api::no_runtime(), state_dir: dir.clone(), default_state_dir: dir, audit_file: false }, cx)
    });
    cx.run_until_parked();
    cx.update(|cx| {
        toasts(cx).update(cx, |t, cx| {
            t.queue.items.clear();
            cx.notify();
        });
        api.update(cx, |s, cx| {
            let now = std::time::SystemTime::now();
            s.status = Status::On("/run/user/1000/devdeck/main.sock".into());
            s.audit_path = Some("/home/u/.local/state/devdeck/api-audit.jsonl".into());
            let terminal = Who::Terminal { workspace: "work".into(), n: 3, uid: "u".into(), shell: 1, shell_start: 1, starts: 1 };
            let long = Who::Outside { pid: Some(4242), program: "a-program-whose-name-is-long-enough-to-be-cut-short-at-the-end-of-the-settings-page-row-when-it-is-narrow".into() };
            s.connections.insert(1, Connection::new(terminal.clone(), Some(4100), now, 12));
            s.connections.insert(2, Connection::new(long, Some(4242), now, 1));
            s.audit.push_front(Entry::new(terminal.label(), Event::Connected, None, None));
            s.audit.push_front(Entry::new("devdeck-ctl (pid 4242)".into(), Event::Denied, Some("run.list".into()), None));
            s.audit.push_front(Entry::new("?".into(), Event::Refused, None, Some("another user (uid 1001)".into())));
            cx.notify();
        });
    });
}

fn states() -> Vec<State> {
    let s = |name, layout, settle_ms, scale, settings_open| State {
        name,
        layout,
        settle: Duration::from_millis(settle_ms),
        scale,
        settings_open,
        extra: None,
        workspaces: None,
    };
    let x = |name, scale, extra: Extra| State {
        name,
        layout: gallery_layout,
        settle: Duration::from_millis(200),
        scale,
        settings_open: false,
        extra: Some(extra),
        workspaces: None,
    };
    let a = |name, scale, extra: Extra| State {
        name,
        layout: devdeck_app::initial_layout,
        settle: Duration::from_millis(200),
        scale,
        settings_open: true,
        extra: Some(extra),
        workspaces: None,
    };
    let w = |name, active, scale, extra: Option<Extra>| State {
        name,
        layout: devdeck_app::initial_layout,
        settle: Duration::from_millis(300),
        scale,
        settings_open: false,
        extra,
        workspaces: Some(active),
    };
    vec![
        s("01-initial", devdeck_app::initial_layout, 500, 1.0, false),
        s("02-logs-50k-lines", devdeck_app::initial_layout, 30_000, 1.0, false),
        s("03-collapsed-in-column", collapsed_column, 500, 1.0, false),
        s("04-collapsed-in-row-rail", collapsed_row, 500, 1.0, false),
        s("05-maximized", maximized, 500, 1.0, false),
        s("06-narrow-panes-long-titles", narrow, 500, 1.0, false),
        s("07-empty", Layout::new, 100, 1.0, false),
        s("08-detached", detached, 500, 1.0, false),
        s("09-no-config-welcome", welcome, 100, 1.0, false),
        s("10-scale-80", devdeck_app::initial_layout, 500, 0.8, false),
        s("11-scale-125", devdeck_app::initial_layout, 500, 1.25, false),
        s("12-scale-150", devdeck_app::initial_layout, 500, 1.5, false),
        s("13-narrow-at-80", narrow, 500, 0.8, false),
        s("14-detached-at-150", detached, 500, 1.5, false),
        s("15-one-tab-focused", one_tab_focused, 500, 1.0, false),
        s("16-one-tab-focused-150", one_tab_focused, 500, 1.5, false),
        s("20-settings-100", devdeck_app::initial_layout, 200, 1.0, true),
        s("21-settings-150", devdeck_app::initial_layout, 200, 1.5, true),
        a("22-settings-api-off", 1.0, api_off),
        a("23-settings-api-callers", 1.0, api_callers),
        a("24-settings-api-callers-150", 1.5, api_callers),
        x("30-gallery-menu-toasts", 1.0, gallery_menu_and_toasts),
        x("31-gallery-menu-toasts-150", 1.5, gallery_menu_and_toasts),
        x("32-gallery-modal", 1.0, gallery_modal),
        w("40-workspace-work", "work", 1.0, None),
        w("41-workspace-personal", "personal", 1.0, None),
        w("42-workspace-menu", "work", 1.0, Some(workspace_menu)),
        w("43-quit-guard", "work", 1.0, Some(quit_guard)),
        w("44-quit-guard-150", "work", 1.5, Some(quit_guard)),
        w("50-apps-pinned-selected", "work", 1.0, Some(apps_pinned)),
        w("51-filter-matches", "work", 1.0, Some(filter_matches)),
        w("52-filter-no-match", "work", 1.0, Some(filter_none)),
        w("53-running-only", "work", 1.0, Some(running_only)),
        w("54-cluster-env-menu", "work", 1.0, Some(cluster_env_menu)),
        w("55-cluster-protected-env", "work", 1.0, Some(cluster_protected)),
        w("56-cluster-list-own-env", "work", 1.0, Some(cluster_list_env)),
        w("56-personal-no-cluster", "personal", 1.0, None),
        w("57-sidebar-hidden", "work", 1.0, Some(sidebar_hidden)),
        w("58-apps-150", "work", 1.5, Some(apps_pinned)),
        w("59-row-menu", "work", 1.0, Some(row_menu)),
        w("60-ports", "work", 1.0, Some(ports)),
        w("61-apps-tree", "work", 1.0, Some(apps_tree)),
        w("62-apps-tree-filtered", "work", 1.0, Some(apps_tree_filtered)),
        w("62b-apps-groups", "work", 1.0, Some(apps_groups)),
        w("66-run-context-menu", "work", 1.0, Some(run_context_menu)),
        w("62c-sections-folded", "work", 1.0, Some(sections_folded)),
        w("62d-sidebar-wide", "work", 1.0, Some(sidebar_wide)),
        w("70-app-view", "work", 1.0, Some(app_view)),
        w("71-app-missing-tool", "work", 1.0, Some(app_missing_tool)),
        w("72-run-streaming", "work", 1.0, Some(run_streaming)),
        w("73-run-exited-0", "work", 1.0, Some(run_exited)),
        w("74-run-crashed-toast", "work", 1.0, Some(run_crashed)),
        w("75-run-not-started", "work", 1.0, Some(run_not_started)),
        w("76-run-terminal-focused", "work", 1.0, Some(run_terminal)),
        w("76b-run-links", "work", 1.0, Some(run_links)),
        w("76c-terminal-tab", "work", 1.0, Some(terminals)),
        w("64-terminals-list", "work", 1.0, Some(terminals_list)),
        w("77-run-args", "work", 1.0, Some(run_args)),
        w("78-run-args-menu", "work", 1.0, Some(run_args_menu)),
        w("79-run-args-typing", "work", 1.0, Some(run_args_typing)),
        w("80-svc-ready", "work", 1.0, Some(svc_ready)),
        w("81-svc-rolling", "work", 1.0, Some(svc_rolling)),
        w("81-svc-redeploying", "work", 1.0, Some(svc_redeploying)),
        w("82-svc-failing", "work", 1.0, Some(svc_failed)),
        w("83-svc-protected-locked", "work", 1.0, Some(svc_protected)),
        w("84-svc-confirm", "work", 1.0, Some(svc_confirm)),
        w("84-svc-confirm-refused", "work", 1.0, Some(svc_confirm_refused)),
        w("84-svc-redeploy-confirm", "work", 1.0, Some(svc_redeploy_confirm)),
        w("85-svc-suggestions", "work", 1.0, Some(svc_suggestions)),
        w("85-svc-tag-ready", "work", 1.0, Some(svc_tag_ready)),
        w("85-svc-notify", "work", 1.0, Some(svc_notify)),
        w("86-svc-loading", "work", 1.0, Some(svc_loading)),
        w("87-svc-unreachable", "work", 1.0, Some(svc_unreachable)),
        w("88-svc-forbidden-narrow", "work", 1.0, Some(svc_forbidden_narrow)),
        w("89-svc-ready-150", "work", 1.5, Some(svc_ready)),
        w("90-logs-earlier-loading", "work", 1.0, Some(logs_earlier_loading)),
        w("90-logs-earlier-start", "work", 1.0, Some(logs_earlier_start)),
        w("90-logs-filter-find", "work", 1.0, Some(logs_find)),
        w("90-logs-json", "work", 1.0, Some(logs_json)),
        w("90-logs-medium", "work", 1.0, Some(logs_medium)),
        w("90-logs-narrow", "work", 1.0, Some(logs_narrow)),
        w("90-logs-refused", "work", 1.0, Some(logs_refused)),
        w("91-pod-crashloop", "work", 1.0, Some(pod)),
        w("92-matrix-highlight", "work", 1.0, Some(matrix)),
        w("94-svc-lost-connection", "work", 1.0, Some(svc_lost_connection)),
        w("95-svc-kube-like", "work", 1.0, Some(svc_kube_like)),
        w("96-cluster-watch", "work", 1.0, Some(cluster_watch)),
        w("97-git-panel", "work", 1.0, Some(git_panel)),
        w("98-git-panel-remotes", "work", 1.0, Some(git_panel_remotes)),
        w("99-git-panel-narrow", "work", 1.0, Some(git_panel_narrow)),
        w("93-port", "work", 1.0, Some(port)),
        w("A0-cheat-sheet", "work", 1.0, Some(cheat_sheet)),
        w("A1-cheat-sheet-filtered", "work", 1.0, Some(cheat_sheet_filtered)),
        w("A2-palette-empty", "work", 1.0, Some(palette_empty)),
        w("A3-palette-matches", "work", 1.0, Some(palette_matches)),
        w("A4-palette-drill", "work", 1.0, Some(palette_drill)),
        w("A5-palette-env", "work", 1.0, Some(palette_env)),
        w("A6-tab-menu", "work", 1.0, Some(tab_menu)),
        w("A6b-tab-menu-places", "work", 1.0, Some(tab_menu_places)),
        w("A7-send-to", "work", 1.0, Some(send_to)),
        w("A8-palette-150", "work", 1.5, Some(palette_matches)),
        w("B0-layout-menu", "work", 1.0, Some(layout_menu)),
        w("B1-layouts-list", "work", 1.0, Some(layouts_list)),
        w("B2-layout-save-error", "work", 1.0, Some(layout_save_error)),
        w("C0-file-rust", "work", 1.0, Some(file_rust)),
        w("C1-file-md-rendered", "work", 1.0, Some(file_md_rendered)),
        w("C2-file-md-source", "work", 1.0, Some(file_md_source)),
        w("C3-file-not-text", "work", 1.0, Some(file_not_text)),
        w("C4-file-missing", "work", 1.0, Some(file_missing)),
        w("C5-file-rust-150", "work", 1.5, Some(file_rust)),
        w("D0-notes-list", "work", 1.0, Some(notes_list)),
        w("D1-note-rendered", "work", 1.0, Some(note_rendered)),
        w("D2-note-source", "work", 1.0, Some(note_source)),
        w("D3-note-delete", "work", 1.0, Some(note_delete)),
        w("E0-bell-clear", "work", 1.0, None),
        w("E1-bell-unread", "work", 1.0, Some(bell_unread)),
        w("E2-bell-errors", "work", 1.0, Some(bell_errors)),
        w("E3-toasts-long", "work", 1.0, Some(toasts_long)),
        w("E4-center-rows", "work", 1.0, Some(center_rows)),
        w("E5-center-rows-150", "work", 1.5, Some(center_rows)),
        w("E6-reveal-flash", "work", 1.0, Some(reveal_flash)),
        w("E7-reveal-flash-still", "work", 1.0, Some(reveal_flash_still)),
    ]
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "target/snapshots".into()));
    let filter = args.next();
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    // Removed when dropped, on every way out (`?` included).
    let config = devdeck_core::testing::TempDir::new("snapshot");
    let text = gpui_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(devdeck_ui::Assets), gpui_platform::current_headless_renderer);
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    for state in states().into_iter().filter(|s| filter.as_ref().is_none_or(|f| f.split(',').any(|part| s.name.contains(part)))) {
        // One app for every state: what a state turned on (reduced motion) goes off.
        cx.update(|cx| cx.set_reduce_motion(false));
        cx.update(|cx| {
            app_settings::set_scale(state.scale, cx);
            app_settings::set_cover(app_settings::Cover::None, cx);
            app_settings::set_open(state.settings_open, cx);
            toasts(cx).update(cx, |t, cx| {
                t.queue.items.clear();
                t.notes.clear();
                cx.notify();
            });
        });
        let mut specs = vec![WorkspaceSpec::new("work", "work", state.layout)];
        if state.workspaces.is_some() {
            specs.push(WorkspaceSpec::new("personal", "personal", devdeck_app::personal_layout));
        }
        let shell = cx.update(|cx| Shell::start(specs, state.workspaces.unwrap_or("work"), None, cx));
        cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
        // The states were made on the Apps list (groups, running dots); a workspace lands on Pulses.
        let sidebars: Vec<_> = cx.update(|cx| shell.read(cx).workspaces.iter().map(|w| w.sidebar.clone()).collect());
        for s in sidebars {
            cx.update(|cx| s.update(cx, |s, cx| s.show(Activity::Apps, cx)));
        }
        cx.run_until_parked();
        cx.advance_clock(state.settle);
        cx.run_until_parked();
        if let Some(extra) = state.extra {
            let main: AnyWindowHandle = cx.update(|cx| shell.read(cx).main_window().expect("main window").into());
            extra(&mut cx, main, &shell);
            cx.run_until_parked();
        }
        // Main window first (w0), then detached ones.
        let windows = cx.update(|cx| {
            let main: AnyWindowHandle = shell.read(cx).main_window().expect("main window").into();
            let mut all = cx.windows();
            all.sort_by_key(|w| *w != main);
            all
        });
        for (i, w) in windows.iter().enumerate() {
            cx.update_window(*w, |_, window, _| window.refresh()).map_err(|e| e.to_string())?;
            cx.run_until_parked();
            let image = cx.capture_screenshot(*w).map_err(|e| e.to_string())?;
            let path = out.join(format!("{}-w{i}.png", state.name));
            image.save(&path).map_err(|e| e.to_string())?;
            println!("{}", path.display());
        }
        for w in windows {
            let _ = cx.update_window(w, |_, window, _| window.remove_window());
        }
        cx.run_until_parked();
        drop(shell);
    }
    Ok(())
}
