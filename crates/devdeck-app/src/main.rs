//! devdeck: the M1 prototype on fake data. Measures itself on request
//! (`DEVDECK_STATS`, `DEVDECK_STRESS`, `DEVDECK_LATENCY`; see docs/WORKFLOW.md).

use devdeck_app::shell::{Shell, try_shell};
use devdeck_app::runner::RunKey;
use devdeck_app::stats;
use devdeck_core::state::{StateStore, default_state_dir};
use devdeck_core::view_addr::ViewAddr;
use devdeck_core::layout::{Drop, Edge, Layout, Result as LayoutResult, ViewRef};
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
            spawn_stress(shell, cycles, cx);
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

type Step = Box<dyn Fn(&mut Layout) -> LayoutResult<()>>;

/// One stress cycle: open a view, split it out, collapse and expand, maximize and
/// restore, every tenth cycle detach and re-attach it (a real OS window), close it.
fn stress_steps(n: u64) -> Vec<Step> {
    // Every view type in turn (step 5), so their timers and handles are cycled too.
    let view = match n % 7 {
        0 => ViewAddr::App { app: "entityservice".into() }.to_ref(),
        1 => ViewAddr::Run { app: "eventservice".into(), cmd: "build".into() }.to_ref(),
        2 => ViewAddr::Svc { env: "dev35".into(), name: "graphservice".into() }.to_ref(),
        3 => ViewAddr::Logs { env: "dev35".into(), name: "graphservice".into() }.to_ref(),
        4 => ViewAddr::Matrix.to_ref(),
        5 => ViewAddr::Port { port: "5720".into() }.to_ref(),
        _ => ViewRef::new(format!("stress/{n}")),
    };
    let at = |l: &Layout, v: &ViewRef| l.find_view(v).ok_or(devdeck_core::layout::LayoutError::TabOutOfRange);
    let mut s: Vec<Step> = Vec::new();
    let v = view.clone();
    s.push(Box::new(move |l| {
        let p = l.main().focused;
        l.open(v.clone(), p).map(|_| ())
    }));
    let v = view.clone();
    s.push(Box::new(move |l| {
        let (p, i) = at(l, &v)?;
        l.move_tab(p, i, Drop::Edge(p, Edge::Right))
    }));
    let v = view.clone();
    s.push(Box::new(move |l| {
        let (p, _) = at(l, &v)?;
        l.toggle_collapse(p).and_then(|_| l.toggle_collapse(p))
    }));
    let v = view.clone();
    s.push(Box::new(move |l| {
        let (p, _) = at(l, &v)?;
        l.toggle_maximize(p).and_then(|_| l.toggle_maximize(p))
    }));
    if n.is_multiple_of(10) {
        let v = view.clone();
        s.push(Box::new(move |l| {
            let (p, i) = at(l, &v)?;
            l.detach(p, i).map(|_| ())
        }));
        let v = view.clone();
        s.push(Box::new(move |l| {
            let (p, _) = at(l, &v)?;
            let w = l.window_of(p)?;
            l.reattach(w).map(|_| ())
        }));
    }
    s.push(Box::new(move |l| l.close_view(&view)));
    s
}

/// Runs stress cycles on the active workspace, printing memory every 500, then quits
/// (bypassing the guard). Besides the layout steps: runs start and stop, every 20th
/// cycle the palette, cheat sheet and a menu open and close, detached windows get a
/// menu before re-attaching, every 25th cycle the workspace switches away and back,
/// every 50th the named layout switches and back. Use a temporary DEVDECK_STATE_DIR.
fn spawn_stress(shell: Entity<Shell>, cycles: u64, cx: &mut App) {
    cx.spawn(async move |cx| {
        let frame = Duration::from_millis(16);
        cx.background_executor().timer(Duration::from_secs(3)).await;
        let m = stats::memory();
        eprintln!("devdeck-stress cycle=0 rss_kb={} pss_kb={} uss_kb={}", m.rss, m.pss, m.uss);
        let deck = cx.update(|cx| shell.read(cx).active_deck());
        let home = cx.update(|cx| shell.read(cx).active);
        let other = cx.update(|cx| (home + 1) % shell.read(cx).workspaces.len());
        let runner = cx.update(|cx| shell.read(cx).workspaces[home].services.read(cx).runner.clone());
        let main = cx.update(|cx| shell.read(cx).main_window()).expect("main window");
        let base_layout = cx.update(|cx| deck.read(cx).book.current.clone());
        let _ = cx.update(|cx| deck.update(cx, |d, cx| d.save_layout_as("stress-b", cx)));
        let _ = cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(&base_layout, cx)));
        let run = RunKey::new("eventservice", "build");
        for n in 1..=cycles {
            if n % 25 == 0 {
                for to in [other, home] {
                    cx.update(|cx| shell.update(cx, |s, cx| s.switch(to, cx)));
                    cx.background_executor().timer(frame).await;
                }
            }
            if n % 50 == 0 {
                for to in ["stress-b", base_layout.as_str()] {
                    let _ = cx.update(|cx| deck.update(cx, |d, cx| d.switch_layout(to, cx)));
                    cx.background_executor().timer(frame).await;
                }
            }
            if n % 20 == 0 {
                let _ = main.update(cx, |_, window, cx| devdeck_app::palette::open(window, cx));
                cx.background_executor().timer(frame).await;
                let _ = main.update(cx, |_, window, cx| devdeck_ui::pop_modal(window, cx));
                let _ = main.update(cx, |_, window, cx| devdeck_app::cheat_sheet::open(window, cx));
                cx.background_executor().timer(frame).await;
                let _ = main.update(cx, |_, window, cx| devdeck_ui::pop_modal(window, cx));
                let _ = main.update(cx, |_, window, cx| devdeck_ui::open_menu(Default::default(), vec![], window, cx));
                cx.background_executor().timer(frame).await;
                close_menu(main.into(), cx);
            }
            if n % 7 == 1 {
                let _ = cx.update(|cx| runner.update(cx, |r, cx| r.start(&run, None, cx)));
            }
            let steps = stress_steps(n);
            let detach_at = steps.len().checked_sub(3);
            for (i, step) in steps.into_iter().enumerate() {
                let ok = cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| step(l)).is_some()));
                if !ok {
                    eprintln!("devdeck-stress cycle={n} step failed");
                }
                cx.background_executor().timer(frame).await;
                // A menu in the detached window before it re-attaches (frees its layer).
                if n.is_multiple_of(10) && Some(i) == detach_at {
                    cx.background_executor().timer(frame).await;
                    let detached = cx.update(|cx| cx.windows().into_iter().find(|w| *w != main.into()));
                    if let Some(w) = detached {
                        let _ = w.update(cx, |_, window, cx| devdeck_ui::open_menu(Default::default(), vec![], window, cx));
                        cx.background_executor().timer(frame).await;
                    }
                }
            }
            if n % 7 == 1 {
                cx.update(|cx| runner.update(cx, |r, cx| r.stop(&run, cx)));
            }
            if n % 500 == 0 || n == cycles {
                let m = stats::memory();
                let (views, leaked, windows, layers, sampling) = cx.update(|cx| {
                    let s = shell.read(cx);
                    let decks: Vec<_> = s.workspaces.iter().map(|w| w.deck.clone()).collect();
                    let sampling = s.workspaces.iter().filter_map(|w| w.services.read(cx).cluster.clone()).map(|c| c.read(cx).samplers()).sum::<usize>();
                    let views: usize = decks.iter().map(|d| d.read(cx).live_views()).sum();
                    let leaked: usize = decks.iter().map(|d| d.update(cx, |d, _| d.leaked_views(3))).sum();
                    (views, leaked, cx.windows().len(), devdeck_ui::overlay::layer_count(cx), sampling)
                });
                eprintln!(
                    "devdeck-stress cycle={n} rss_kb={} pss_kb={} uss_kb={} views={views} leaked_views={leaked} windows={windows} layers={layers} samplers={sampling}",
                    m.rss, m.pss, m.uss
                );
            }
        }
        cx.update(|cx| {
            shell.update(cx, |s, cx| s.flush(cx));
            cx.quit()
        });
    })
    .detach();
}

fn close_menu(window: gpui::AnyWindowHandle, cx: &mut gpui::AsyncApp) {
    cx.update(|cx| {
        let layer = devdeck_ui::overlay::layer(window, cx);
        layer.update(cx, |l, cx| {
            l.menu = None;
            cx.notify();
        });
    });
}
