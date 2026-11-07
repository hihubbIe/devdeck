//! Renders devdeck states offscreen with the real GPU renderer and text system, and
//! writes one PNG per window per state. Used for the visual check step of every change
//! (docs/WORKFLOW.md). Settings are isolated in a temporary config folder.
//!
//! `cargo run -p devdeck-app --features snapshot --bin devdeck-snapshot -- <out-dir> [filter]`
//! where `filter` is a comma-separated list of name fragments (`50-,51-`).

use devdeck_app::shell::{Shell, WorkspaceSpec};
use devdeck_app::runner::RunKey;
use devdeck_app::sidebar::{Activity, Sidebar};
use devdeck_app::views::app_view::AppView;
use devdeck_app::views::service_view::ServiceView;
use devdeck_core::view_addr::ViewAddr;
use devdeck_app::{app_settings, gallery::Gallery, window_view::workspace_menu_items};
use devdeck_core::layout::{Drop, Edge, Layout, ViewRef};
use devdeck_ui::{ToastKind, logical, open_menu, toasts};
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
            (ToastKind::Info, "Layout: Geo work."),
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
            s.set_env("hades", cx);
        })
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
        let items = Sidebar::item_menu(&s, "boardservice", false);
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

fn run_cmd(cx: &mut HeadlessAppContext, shell: &Entity<Shell>, app: &str, cmd: &str, wait_ms: u64) {
    let runner = cx.update(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).runner.clone());
    cx.update(|cx| runner.update(cx, |r, cx| r.start(&RunKey::new(app, cmd), Some("dev35".into()), cx))).expect("starts");
    cx.advance_clock(Duration::from_millis(wait_ms));
    cx.run_until_parked();
}

fn app_view(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    run_cmd(cx, shell, "pagerendererservice", "test", 1200);
    run_cmd(cx, shell, "pagerendererservice", "run", 1000);
    open_view(cx, shell, ViewAddr::App { app: "pagerendererservice".into() });
    let deck = cx.update(|cx| shell.read(cx).active_deck());
    let v = cx.update(|cx| deck.read(cx).view(&ViewAddr::App { app: "pagerendererservice".into() }.to_ref()).unwrap().downcast::<AppView>().unwrap());
    cx.update(|cx| {
        v.update(cx, |v, cx| {
            v.show_hidden = true;
            v.selected = Some("build".into());
            cx.notify();
        })
    });
}

fn app_missing_tool(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, ViewAddr::App { app: "remotedesktopapp".into() });
}

fn run_streaming(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, ViewAddr::Run { app: "entityservice".into(), cmd: "run".into() });
    cx.advance_clock(Duration::from_secs(9));
    cx.run_until_parked();
}

fn run_exited(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    run_cmd(cx, shell, "eventservice", "build", 1000);
    open_view(cx, shell, ViewAddr::Run { app: "eventservice".into(), cmd: "build".into() });
}

fn run_crashed(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    run_cmd(cx, shell, "boardservice", "run", 4200);
    open_view(cx, shell, ViewAddr::Run { app: "boardservice".into(), cmd: "run".into() });
}

fn run_not_started(cx: &mut HeadlessAppContext, _w: AnyWindowHandle, shell: &Entity<Shell>) {
    open_view(cx, shell, ViewAddr::Run { app: "eventservice".into(), cmd: "test".into() });
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
            c.deployments(env, cx);
        })
    });
    cx.advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update(|cx| c.update(cx, |c, cx| c.set_image(env, name, tag, true, cx))).expect("deploys");
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

fn svc_suggestions(cx: &mut HeadlessAppContext, w: AnyWindowHandle, shell: &Entity<Shell>) {
    deploy(cx, shell, "dev35", "entityservice", "lan-160", 3500);
    let v = service(cx, shell, "dev35", "entityservice", true);
    let items = cx.update(|cx| ServiceView::suggestions_for(&v, cx));
    let _ = cx.update_window(w, |_, window, cx| open_menu(point(logical(720.), logical(140.)), items, window, cx));
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

fn maximized() -> Layout {
    let mut l = devdeck_app::initial_layout();
    let p = pane_of(&l, "demo/deployments");
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
        s("10-scale-80", devdeck_app::initial_layout, 500, 0.8, false),
        s("11-scale-125", devdeck_app::initial_layout, 500, 1.25, false),
        s("12-scale-150", devdeck_app::initial_layout, 500, 1.5, false),
        s("13-narrow-at-80", narrow, 500, 0.8, false),
        s("14-detached-at-150", detached, 500, 1.5, false),
        s("20-settings-100", devdeck_app::initial_layout, 200, 1.0, true),
        s("21-settings-150", devdeck_app::initial_layout, 200, 1.5, true),
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
        w("56-personal-no-cluster", "personal", 1.0, None),
        w("57-sidebar-hidden", "work", 1.0, Some(sidebar_hidden)),
        w("58-apps-150", "work", 1.5, Some(apps_pinned)),
        w("59-row-menu", "work", 1.0, Some(row_menu)),
        w("60-ports", "work", 1.0, Some(ports)),
        w("70-app-view", "work", 1.0, Some(app_view)),
        w("71-app-missing-tool", "work", 1.0, Some(app_missing_tool)),
        w("72-run-streaming", "work", 1.0, Some(run_streaming)),
        w("73-run-exited-0", "work", 1.0, Some(run_exited)),
        w("74-run-crashed-toast", "work", 1.0, Some(run_crashed)),
        w("75-run-not-started", "work", 1.0, Some(run_not_started)),
        w("80-svc-ready", "work", 1.0, Some(svc_ready)),
        w("81-svc-rolling", "work", 1.0, Some(svc_rolling)),
        w("82-svc-failed-undo", "work", 1.0, Some(svc_failed)),
        w("83-svc-protected-locked", "work", 1.0, Some(svc_protected)),
        w("84-svc-confirm", "work", 1.0, Some(svc_confirm)),
        w("85-svc-suggestions", "work", 1.0, Some(svc_suggestions)),
        w("86-svc-loading", "work", 1.0, Some(svc_loading)),
        w("87-svc-unreachable", "work", 1.0, Some(svc_unreachable)),
        w("88-svc-forbidden-narrow", "work", 1.0, Some(svc_forbidden_narrow)),
        w("89-svc-ready-150", "work", 1.5, Some(svc_ready)),
    ]
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let out = PathBuf::from(args.next().unwrap_or_else(|| "target/snapshots".into()));
    let filter = args.next();
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let config = std::env::temp_dir().join(format!("devdeck-snapshot-{}", std::process::id()));
    let text = gpui_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(()), gpui_platform::current_headless_renderer);
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    for state in states().into_iter().filter(|s| filter.as_ref().is_none_or(|f| f.split(',').any(|part| s.name.contains(part)))) {
        cx.update(|cx| {
            app_settings::set_scale(state.scale, cx);
            app_settings::set_open(state.settings_open, cx);
            toasts(cx).update(cx, |t, cx| {
                t.queue.items.clear();
                cx.notify();
            });
        });
        let mut specs = vec![WorkspaceSpec::new("work", "work", state.layout)];
        if state.workspaces.is_some() {
            specs.push(WorkspaceSpec::new("personal", "personal", devdeck_app::personal_layout));
        }
        let shell = cx.update(|cx| Shell::start(specs, state.workspaces.unwrap_or("work"), None, cx));
        cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
        cx.run_until_parked();
        cx.advance_clock(state.settle);
        cx.run_until_parked();
        // Main window first (w0), then detached ones.
        let windows = cx.update(|cx| {
            let main: AnyWindowHandle = shell.read(cx).main_window().expect("main window").into();
            let mut all = cx.windows();
            all.sort_by_key(|w| *w != main);
            all
        });
        if let Some(extra) = state.extra {
            extra(&mut cx, windows[0], &shell);
            cx.run_until_parked();
        }
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
    let _ = std::fs::remove_dir_all(&config);
    Ok(())
}
