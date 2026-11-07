//! devdeck: the M1 prototype on fake data. Measures itself on request
//! (`DEVDECK_STATS`, `DEVDECK_STRESS`, `DEVDECK_LATENCY`; see docs/WORKFLOW.md).

use devdeck_app::shell::{Shell, try_shell};
use devdeck_app::stats;
use devdeck_core::state::{StateStore, default_state_dir};
use gpui::{App, Entity};
use std::time::Duration;

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        devdeck_app::init(cx);
        // Fallback only: every normal quit goes through the shell's guard.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                if let Some(s) = try_shell(cx) {
                    s.update(cx, |s, cx| s.flush(cx));
                }
                cx.quit();
            }
        })
        .detach();

        let store = StateStore::new(default_state_dir());
        let shell = Shell::start(devdeck_app::demo_workspaces(), "work", Some(store), cx);
        cx.activate(true);

        if let Some(secs) = env_u64("DEVDECK_STATS") {
            spawn_stats_printer(shell.clone(), Duration::from_secs(secs.max(1)), cx);
        }
        #[cfg(feature = "profile")]
        if std::env::var("DEVDECK_LATENCY").is_ok() {
            spawn_latency_printer(cx);
        }
        if let Some(cycles) = env_u64("DEVDECK_STRESS") {
            devdeck_app::stress::spawn(shell, cycles, cx);
        }
    });
}

/// Prints input-to-frame latency from GPUI's profiler every 5 s (profile builds).
/// Excludes compositor and scanout time.
#[cfg(feature = "profile")]
fn spawn_latency_printer(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let lines = cx.update(|cx| {
                cx.windows()
                    .into_iter()
                    .filter_map(|w| w.update(cx, |_, window, _| window.input_latency_snapshot()).ok())
                    .map(|s| {
                        let h = &s.latency_histogram;
                        let ms = |ns: u64| ns as f64 / 1e6;
                        format!(
                            "devdeck-latency samples={} p50_ms={:.2} p95_ms={:.2} p99_ms={:.2} max_ms={:.2}",
                            h.len(),
                            ms(h.value_at_quantile(0.5)),
                            ms(h.value_at_quantile(0.95)),
                            ms(h.value_at_quantile(0.99)),
                            ms(h.max())
                        )
                    })
                    .collect::<Vec<_>>()
            });
            for l in lines {
                eprintln!("{l}");
            }
        }
    })
    .detach();
}

fn spawn_stats_printer(shell: Entity<Shell>, every: Duration, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut last = stats::renders();
        loop {
            cx.background_executor().timer(every).await;
            let now = stats::renders();
            let m = stats::memory();
            let (views, windows, layers) = cx.update(|cx| {
                let s = shell.read(cx);
                let views: usize = s.workspaces.iter().map(|w| w.deck.read(cx).live_views()).sum();
                (views, cx.windows().len(), devdeck_ui::overlay::layer_count(cx))
            });
            eprintln!(
                "devdeck-stats renders_delta={} rss_kb={} pss_kb={} uss_kb={} views={views} windows={windows} layers={layers}",
                now - last,
                m.rss,
                m.pss,
                m.uss
            );
            last = now;
        }
    })
    .detach();
}
