//! devdeck. The workspaces come from the config folder (`workspaces/*.toml`, see
//! `core::config`); `devdeck --demo` runs the demo on fake data instead, with its own
//! state. Measures itself on request (`DEVDECK_STATS`, `DEVDECK_STRESS`, which implies
//! the demo, `DEVDECK_LATENCY`; see docs/WORKFLOW.md).
//! `DEVDECK_OPEN=<view>` opens a view at start (`run/entityservice/go:run`), and
//! `DEVDECK_FAKE_RATE` makes the demo's servers write that many lines per second.
//! `DEVDECK_LOG=error|info|debug|trace|off` changes what is logged on stderr (warnings
//! and errors by default).

use devdeck_app::shell::{Shell, try_shell};
use devdeck_app::stats;
use devdeck_app::shell::WorkspaceSpec;
use devdeck_core::config::{load_machine, load_workspaces};
use devdeck_core::layout::{Layout, ViewRef};
use devdeck_core::settings::default_config_dir;
use devdeck_core::state::{StateStore, default_state_dir};
use gpui::{App, Entity};
use std::time::Duration;

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

fn main() {
    // SAFETY: first thing, before anything starts a thread that could read the
    // environment.
    devdeck_app::scrub_env(|name| unsafe { std::env::remove_var(name) });
    devdeck_app::logger::install();
    let demo = std::env::args().any(|a| a == "--demo") || std::env::var("DEVDECK_STRESS").is_ok();
    // The demo keeps its own state, apart from the real workspaces' layouts and pins.
    let state_dir = if demo { default_state_dir().join("demo") } else { default_state_dir() };
    // One devdeck per state folder. Held by `main` itself: the closure given to `run` is
    // dropped once the app is set up, while `run` blocks until devdeck quits.
    let _lock = match devdeck_app::lifecycle::claim(&state_dir) {
        Ok(lock) => lock,
        Err(why) => {
            eprintln!("devdeck: {why}");
            std::process::exit(1);
        }
    };
    // Whether runs can get scopes (R8.2): checked now, so the first run doesn't wait.
    std::thread::spawn(|| devdeck_sys::scope::systemd_run().is_some());
    gpui_platform::application().with_assets(devdeck_ui::Assets).run(move |cx: &mut App| {
        devdeck_app::init(cx);
        // Fallback only (windows closed from outside): every normal quit goes through the
        // shell's guard. Nothing it started outlives it (R8.2).
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                if let Some(s) = try_shell(cx) {
                    s.update(cx, |s, cx| {
                        s.kill_everything(cx);
                        s.flush(cx);
                    });
                }
                cx.quit();
            }
        })
        .detach();

        let shell = if demo {
            Shell::start(devdeck_app::demo_workspaces(), "work", Some(StateStore::new(state_dir.clone())), cx)
        } else {
            let dir = default_config_dir();
            let (configs, mut errors) = load_workspaces(&dir);
            let (machine, machine_errors) = load_machine(&dir);
            errors.extend(machine_errors);
            let env = devdeck_app::environment::load(machine, cx);
            env.update(cx, |e, _| e.config_problems = errors.iter().map(ToString::to_string).collect());
            let mut specs: Vec<WorkspaceSpec> = configs.into_iter().map(WorkspaceSpec::from_config).collect();
            if specs.is_empty() {
                specs.push(WorkspaceSpec::new("welcome", "welcome", welcome_layout));
            }
            let first = specs.iter().map(|s| s.name.to_lowercase()).min().unwrap_or_default();
            let first = specs.iter().find(|s| s.name.to_lowercase() == first).map(|s| s.id.clone()).unwrap_or_default();
            let shell = Shell::start(specs, &first, Some(StateStore::new(state_dir.clone())), cx);
            devdeck_app::lifecycle::install(&shell, devdeck_core::process::GroupLog::new(&state_dir), cx);
            if let Some(w) = devdeck_app::watcher::start(&shell, dir, cx) {
                devdeck_app::watcher::keep(w, cx);
            }
            shell
        };
        // The control API (R13): after the shell, which it reads.
        devdeck_app::api::start(
            devdeck_app::api::Config {
                runtime: devdeck_sys::dirs::default_runtime().map_err(|e| e.to_string()),
                state_dir: state_dir.clone(),
                default_state_dir: devdeck_core::state::state_dir(|k| if k == "DEVDECK_STATE_DIR" { None } else { std::env::var(k).ok() }),
                audit_file: true,
            },
            cx,
        );
        cx.activate(true);
        if let Ok(view) = std::env::var("DEVDECK_OPEN") {
            let deck = shell.read(cx).active_deck().downgrade();
            devdeck_app::views::open_in(&deck, devdeck_core::view_addr::ViewAddr::parse(&devdeck_core::layout::ViewRef::new(view)), cx);
        }

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

/// The first-run screen, when no workspace is configured.
fn welcome_layout() -> Layout {
    let mut l = Layout::new();
    let p = l.main().focused;
    l.open(ViewRef::new("welcome"), p).expect("open the welcome view");
    l
}

fn spawn_stats_printer(shell: Entity<Shell>, every: Duration, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut last = stats::renders();
        loop {
            cx.background_executor().timer(every).await;
            let now = stats::renders();
            let m = stats::memory();
            let (views, windows, layers, watched, envs) = cx.update(|cx| {
                let s = shell.read(cx);
                let views: usize = s.workspaces.iter().map(|w| w.deck.read(cx).live_views()).sum();
                // The cluster envs watched, of all the workspaces' envs.
                let clusters: Vec<_> = s.workspaces.iter().filter_map(|w| w.services.read(cx).cluster.clone()).collect();
                let watched: usize = clusters.iter().map(|c| c.read(cx).wanted().len()).sum();
                let envs: usize = clusters.iter().map(|c| c.read(cx).envs.len()).sum();
                (views, cx.windows().len(), devdeck_ui::overlay::layer_count(cx), watched, envs)
            });
            eprintln!(
                "devdeck-stats renders_delta={} rss_kb={} pss_kb={} uss_kb={} views={views} windows={windows} layers={layers} envs_watched={watched}/{envs}",
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
