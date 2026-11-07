//! Renders devdeck states offscreen with the real GPU renderer and text system, and
//! writes one PNG per window per state. Used for the visual check step of every change
//! (docs/WORKFLOW.md). Settings are isolated in a temporary config folder.
//!
//! `cargo run -p devdeck-app --features snapshot --bin devdeck-snapshot -- <out-dir> [filter]`

use devdeck_app::shell::{Shell, WorkspaceSpec};
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
    for state in states().into_iter().filter(|s| filter.as_ref().is_none_or(|f| s.name.contains(f.as_str()))) {
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
