//! Renders devdeck states offscreen with the real GPU renderer and text system, and
//! writes one PNG per window per state. Used for the visual check step of every change
//! (docs/WORKFLOW.md). Settings are isolated in a temporary config folder.
//!
//! `cargo run -p devdeck-app --features snapshot --bin devdeck-snapshot -- <out-dir> [filter]`

use devdeck_app::{app_settings, deck::Deck};
use devdeck_core::layout::{Drop, Edge, Layout, ViewRef};
use gpui::{AppContext, HeadlessAppContext};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

struct State {
    name: &'static str,
    layout: fn() -> Layout,
    settle: Duration,
    scale: f32,
    settings_open: bool,
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
        });
        let deck = cx.update(|cx| cx.new(|_| Deck::new((state.layout)())));
        cx.update(|cx| deck.update(cx, |d, cx| d.sync(cx)));
        cx.run_until_parked();
        cx.advance_clock(state.settle);
        cx.run_until_parked();
        let windows = cx.update(|cx| cx.windows());
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
        drop(deck);
    }
    let _ = std::fs::remove_dir_all(&config);
    Ok(())
}
