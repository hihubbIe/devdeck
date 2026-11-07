//! devdeck: M0 spike. Proves the pane engine, multi-window, a streaming view and a
//! background cluster feed on GPUI, and measures itself (see docs/plans/M0-spike.md).

use devdeck_app::deck::Deck;
use devdeck_app::stats;
use devdeck_core::layout::{Drop, Edge, Layout, Result as LayoutResult, ViewRef};
use gpui::{App, AppContext, Entity};
use std::time::Duration;

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        devdeck_app::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let deck = cx.new(|_| Deck::new(devdeck_app::initial_layout()));
        deck.update(cx, |d, cx| d.sync(cx));
        cx.activate(true);

        if let Some(secs) = env_u64("DEVDECK_STATS") {
            spawn_stats_printer(deck.clone(), Duration::from_secs(secs.max(1)), cx);
        }
        if let Some(cycles) = env_u64("DEVDECK_STRESS") {
            spawn_stress(deck, cycles, cx);
        }
    });
}

fn spawn_stats_printer(deck: Entity<Deck>, every: Duration, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut last = stats::renders();
        loop {
            cx.background_executor().timer(every).await;
            let now = stats::renders();
            let m = stats::memory();
            let views = cx.update(|cx| deck.read(cx).live_views());
            eprintln!(
                "devdeck-stats renders_delta={} rss_kb={} pss_kb={} uss_kb={} views={views}",
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
    let view = ViewRef::new(format!("stress/{n}"));
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
            let target = l.main().focused;
            l.attach(w, Drop::Into(target))
        }));
    }
    s.push(Box::new(move |l| l.close_view(&view)));
    s
}

/// Runs stress cycles, printing memory every 500, then quits.
fn spawn_stress(deck: Entity<Deck>, cycles: u64, cx: &mut App) {
    cx.spawn(async move |cx| {
        let frame = Duration::from_millis(16);
        cx.background_executor().timer(Duration::from_secs(3)).await;
        let m = stats::memory();
        eprintln!("devdeck-stress cycle=0 rss_kb={} pss_kb={} uss_kb={}", m.rss, m.pss, m.uss);
        for n in 1..=cycles {
            for step in stress_steps(n) {
                let ok = cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| step(l)).is_some()));
                if !ok {
                    eprintln!("devdeck-stress cycle={n} step failed");
                }
                cx.background_executor().timer(frame).await;
            }
            if n % 500 == 0 || n == cycles {
                let m = stats::memory();
                let leaked = cx.update(|cx| deck.update(cx, |d, _| d.leaked_views(3)));
                let views = cx.update(|cx| deck.read(cx).live_views());
                eprintln!(
                    "devdeck-stress cycle={n} rss_kb={} pss_kb={} uss_kb={} views={views} leaked_views={leaked}",
                    m.rss, m.pss, m.uss
                );
            }
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}
