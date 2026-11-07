//! Renders devdeck layout states offscreen with the real GPU renderer and text
//! system, and writes one PNG per window per state. Used for the visual check step
//! of every change (docs/WORKFLOW.md).
//!
//! `cargo run -p devdeck-app --features snapshot --bin devdeck-snapshot -- <out-dir>`

use devdeck_app::deck::Deck;
use devdeck_core::layout::{Drop, Edge, Layout, ViewRef};
use gpui::{AppContext, HeadlessAppContext};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

type State = (&'static str, fn() -> Layout, Duration);

fn pane_of(l: &Layout, v: &str) -> devdeck_core::layout::PaneId {
    l.find_view(&ViewRef::new(v)).expect("view open").0
}

fn states() -> Vec<State> {
    vec![
        ("01-initial", devdeck_app::initial_layout, Duration::from_millis(500)),
        ("02-logs-50k-lines", devdeck_app::initial_layout, Duration::from_secs(30)),
        ("03-collapsed-in-column", || {
            let mut l = devdeck_app::initial_layout();
            let p = pane_of(&l, "demo/notes");
            l.toggle_collapse(p).unwrap();
            l
        }, Duration::from_millis(500)),
        ("04-collapsed-in-row-rail", || {
            let mut l = devdeck_app::initial_layout();
            let p = pane_of(&l, "demo/logs");
            l.toggle_collapse(p).unwrap();
            l
        }, Duration::from_millis(500)),
        ("05-maximized", || {
            let mut l = devdeck_app::initial_layout();
            let p = pane_of(&l, "demo/deployments");
            l.toggle_maximize(p).unwrap();
            l
        }, Duration::from_millis(500)),
        ("06-narrow-panes-long-titles", || {
            let mut l = devdeck_app::initial_layout();
            let p = pane_of(&l, "demo/logs");
            for v in ["demo/a-very-long-view-name-that-must-truncate", "demo/b-another-long-one-for-truncation", "demo/c"] {
                l.open(ViewRef::new(v), p).unwrap();
                let i = l.pane(p).unwrap().tabs.len() - 1;
                l.move_tab(p, i, Drop::Edge(p, Edge::Right)).unwrap();
            }
            l
        }, Duration::from_millis(500)),
        ("07-empty", Layout::new, Duration::from_millis(100)),
        ("08-detached", || {
            let mut l = devdeck_app::initial_layout();
            let (p, i) = l.find_view(&ViewRef::new("demo/notes")).unwrap();
            l.detach(p, i).unwrap();
            l
        }, Duration::from_millis(500)),
    ]
}

fn main() -> anyhow_free::Result {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "target/snapshots".into()));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let text = gpui_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text, Arc::new(()), gpui_platform::current_headless_renderer);
    cx.update(devdeck_app::init);
    for (name, layout, settle) in states() {
        let deck = cx.update(|cx| cx.new(|_| Deck::new(layout())));
        cx.update(|cx| deck.update(cx, |d, cx| d.sync(cx)));
        cx.run_until_parked();
        cx.advance_clock(settle);
        cx.run_until_parked();
        let windows = cx.update(|cx| cx.windows());
        for (i, w) in windows.iter().enumerate() {
            cx.update_window(*w, |_, window, _| window.refresh()).map_err(|e| e.to_string())?;
            cx.run_until_parked();
            let image = cx.capture_screenshot(*w).map_err(|e| e.to_string())?;
            let path = out.join(format!("{name}-w{i}.png"));
            image.save(&path).map_err(|e| e.to_string())?;
            println!("{}", path.display());
        }
        for w in windows {
            let _ = cx.update_window(w, |_, window, _| window.remove_window());
        }
        cx.run_until_parked();
        drop(deck);
    }
    Ok(())
}

mod anyhow_free {
    pub type Result = std::result::Result<(), String>;
}
