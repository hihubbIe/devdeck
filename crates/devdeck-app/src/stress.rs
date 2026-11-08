//! The leak run (`DEVDECK_STRESS=<cycles>`, R11.4). Every cycle does one of everything
//! the app does, so a short run already goes through every path:
//! - opens each view type (app, run, service, service logs, matrix, port, a demo view) and
//!   presses Esc in an input inside the service view;
//! - starts and stops a run;
//! - splits, collapses, maximizes, detaches (a real OS window, with a menu opened in it)
//!   and re-attaches a tab;
//! - opens and closes the palette, the cheat sheet and a menu in the main window;
//! - switches the workspace and the named layout away and back;
//! - closes what it opened.
//!
//! It prints memory and engine counters every `DEVDECK_STRESS_REPORT` cycles (default
//! 250) and at the end, then quits without the guard. `DEVDECK_STRESS_EXIT=hard` skips
//! teardown, so a heap profiler counts everything still alive (diff two run lengths to
//! see what grows per cycle). Run it with temporary `DEVDECK_STATE_DIR` and
//! `DEVDECK_CONFIG_DIR` (`scripts/measure.py` does).

use crate::deck::Deck;
use crate::runner::RunKey;
use crate::shell::Shell;
use crate::stats;
use devdeck_core::layout::{Drop, Edge, Layout, ViewRef};
use devdeck_core::view_addr::ViewAddr;
use gpui::{AnyWindowHandle, App, AsyncApp, Entity, Keystroke};
use std::time::Duration;

/// Time given to each step so frames render in between.
const FRAME: Duration = Duration::from_millis(16);
const LAYOUT_B: &str = "stress-b";

/// Index of the service view in [`views`]: it has an input to press Esc in.
const SERVICE: usize = 2;

/// The addresses a cycle opens, one per view type.
fn views(n: u64) -> Vec<ViewRef> {
    vec![
        ViewAddr::App { app: "entityservice".into() }.to_ref(),
        ViewAddr::Run { app: "eventservice".into(), cmd: "go:build".into() }.to_ref(),
        ViewAddr::Svc { env: "dev35".into(), name: "graphservice".into() }.to_ref(),
        ViewAddr::Logs { env: "dev35".into(), name: "graphservice".into() }.to_ref(),
        // A pod that isn't there: it still takes its deployment's pods while open.
        ViewAddr::Pod { env: "dev35".into(), pod: "graphservice-0-0".into() }.to_ref(),
        ViewAddr::Matrix.to_ref(),
        ViewAddr::Port { port: "5720".into() }.to_ref(),
        ViewRef::new(format!("stress/{n}")),
    ]
}

pub fn spawn(shell: Entity<Shell>, cycles: u64, cx: &mut App) {
    let report = std::env::var("DEVDECK_STRESS_REPORT").ok().and_then(|s| s.parse().ok()).unwrap_or(250u64).max(1);
    let hard_exit = std::env::var("DEVDECK_STRESS_EXIT").is_ok_and(|v| v == "hard");
    cx.spawn(async move |cx| {
        cx.background_executor().timer(Duration::from_secs(3)).await;
        let m = stats::memory();
        eprintln!("devdeck-stress cycle=0 rss_kb={} pss_kb={} uss_kb={}", m.rss, m.pss, m.uss);
        let s = Stress::new(shell.clone(), cx);
        s.prepare(cx).await;
        for n in 1..=cycles {
            s.cycle(n, cx).await;
            if n % report == 0 || n == cycles {
                s.report(n, cx);
            }
        }
        // Let frames and GPU work in flight finish, so a heap snapshot at exit doesn't
        // depend on how many frames happened to be queued.
        cx.background_executor().timer(Duration::from_millis(500)).await;
        cx.update(|cx| {
            shell.update(cx, |s, cx| s.flush(cx));
            if hard_exit {
                std::process::exit(0);
            }
            cx.quit()
        });
    })
    .detach();
}

struct Stress {
    shell: Entity<Shell>,
    deck: Entity<Deck>,
    main: AnyWindowHandle,
    home: usize,
    other: usize,
    layout_a: String,
}

impl Stress {
    fn new(shell: Entity<Shell>, cx: &mut AsyncApp) -> Self {
        cx.update(|cx| {
            let s = shell.read(cx);
            let home = s.active;
            Self {
                deck: s.active_deck(),
                main: s.main_window().expect("main window").into(),
                home,
                other: (home + 1) % s.workspaces.len(),
                layout_a: s.active_deck().read(cx).book.current.clone(),
                shell: shell.clone(),
            }
        })
    }

    async fn frame(&self, cx: &mut AsyncApp) {
        cx.background_executor().timer(FRAME).await;
    }

    fn apply(&self, cx: &mut AsyncApp, n: u64, what: &str, op: impl FnOnce(&mut Layout) -> devdeck_core::layout::Result<()>) {
        let ok = cx.update(|cx| self.deck.update(cx, |d, cx| d.apply(cx, op).is_some()));
        if !ok {
            eprintln!("devdeck-stress cycle={n} step failed: {what}");
        }
    }

    /// A second named layout to switch to every cycle, and the cluster env loaded so the
    /// service view has its editor from the first cycle.
    async fn prepare(&self, cx: &mut AsyncApp) {
        let cluster = cx.update(|cx| self.shell.read(cx).workspaces[self.home].services.read(cx).cluster.clone());
        if let Some(c) = cluster {
            cx.update(|cx| {
                c.update(cx, |c, cx| {
                    c.want("dev35", cx);
                })
            });
            cx.background_executor().timer(crate::cluster::LOAD_DELAY * 2).await;
        }
        cx.update(|cx| {
            self.deck.update(cx, |d, cx| {
                let _ = d.save_layout_as(LAYOUT_B, cx);
                let _ = d.switch_layout(&self.layout_a, cx);
            })
        });
        self.frame(cx).await;
    }

    async fn cycle(&self, n: u64, cx: &mut AsyncApp) {
        let opened = views(n);
        for (i, v) in opened.iter().enumerate() {
            let v = v.clone();
            self.apply(cx, n, "open", move |l| {
                let p = l.main().focused;
                l.open(v, p).map(|_| ())
            });
            self.frame(cx).await;
            if i == SERVICE {
                self.escape_from_an_input(n, cx).await;
            }
        }
        self.run_once(cx).await;
        self.watch_pods(cx).await;
        self.layout_moves(n, opened.last().expect("a demo view").clone(), cx).await;
        self.overlays(cx).await;
        for to in [self.other, self.home] {
            cx.update(|cx| self.shell.update(cx, |s, cx| s.switch(to, cx)));
            self.frame(cx).await;
        }
        let layout_b = cx.update(|cx| self.deck.read(cx).book.id_named(LAYOUT_B)).unwrap_or_default();
        for to in [layout_b.as_str(), self.layout_a.as_str()] {
            let _ = cx.update(|cx| self.deck.update(cx, |d, cx| d.switch_layout(to, cx)));
            self.frame(cx).await;
        }
        for v in opened {
            self.apply(cx, n, "close", move |l| l.close_view(&v));
        }
        self.frame(cx).await;
    }

    /// The Cluster list's **watch** on, then off: its env-wide pod watch comes and goes.
    async fn watch_pods(&self, cx: &mut AsyncApp) {
        let sidebar = cx.update(|cx| self.shell.read(cx).active_sidebar());
        cx.update(|cx| {
            sidebar.update(cx, |s, cx| {
                let back = s.activity();
                s.show(crate::sidebar::Activity::Cluster, cx);
                s.toggle_watch_pods(cx);
                s.toggle_watch_pods(cx);
                s.show(back, cx);
            })
        });
        self.frame(cx).await;
    }

    /// `t` focuses the service view's tag editor, Esc must give the keys back to the
    /// view (the engine's path out of any input in a view).
    async fn escape_from_an_input(&self, n: u64, cx: &mut AsyncApp) {
        let key = |k: &'static str, cx: &mut AsyncApp| {
            self.main
                .update(cx, |_, window, cx| {
                    window.dispatch_keystroke(Keystroke::parse(k).expect("key"), cx);
                    window.focused(cx)
                })
                .ok()
                .flatten()
        };
        let in_input = key("t", cx);
        self.frame(cx).await;
        let back = key("escape", cx);
        self.frame(cx).await;
        if in_input.is_none() || back.is_none() || in_input == back {
            eprintln!("devdeck-stress cycle={n} step failed: esc from the tag editor");
        }
    }

    async fn run_once(&self, cx: &mut AsyncApp) {
        let runner = cx.update(|cx| self.shell.read(cx).workspaces[self.home].services.read(cx).runner.clone());
        let key = RunKey::new("eventservice", "go:build");
        let _ = cx.update(|cx| runner.update(cx, |r, cx| r.start(&key, None, cx)));
        self.frame(cx).await;
        cx.update(|cx| runner.update(cx, |r, cx| r.stop(&key, cx)));
    }

    /// Split, collapse, maximize, detach (with a menu in that window) and re-attach.
    async fn layout_moves(&self, n: u64, v: ViewRef, cx: &mut AsyncApp) {
        let at = |l: &Layout, v: &ViewRef| l.find_view(v).ok_or(devdeck_core::layout::LayoutError::TabOutOfRange);
        let v1 = v.clone();
        self.apply(cx, n, "split", move |l| {
            let (p, i) = at(l, &v1)?;
            l.move_tab(p, i, Drop::Edge(p, Edge::Right))
        });
        self.frame(cx).await;
        // One change per step, so visibility really goes away and comes back.
        for what in ["collapse", "expand", "maximize", "restore"] {
            let v = v.clone();
            self.apply(cx, n, what, move |l| {
                let (p, _) = at(l, &v)?;
                if what == "collapse" || what == "expand" { l.toggle_collapse(p) } else { l.toggle_maximize(p) }
            });
            self.frame(cx).await;
        }
        let v4 = v.clone();
        self.apply(cx, n, "detach", move |l| {
            let (p, i) = at(l, &v4)?;
            l.detach(p, i).map(|_| ())
        });
        self.frame(cx).await;
        self.frame(cx).await;
        let detached = cx.update(|cx| cx.windows().into_iter().find(|w| *w != self.main));
        if let Some(w) = detached {
            let _ = w.update(cx, |_, window, cx| devdeck_ui::open_menu(Default::default(), vec![], window, cx));
            self.frame(cx).await;
        }
        self.apply(cx, n, "re-attach", move |l| {
            let (p, _) = at(l, &v)?;
            let w = l.window_of(p)?;
            l.reattach(w).map(|_| ())
        });
        self.frame(cx).await;
    }

    async fn overlays(&self, cx: &mut AsyncApp) {
        let _ = self.main.update(cx, |_, window, cx| crate::palette::open(window, cx));
        self.frame(cx).await;
        let _ = self.main.update(cx, |_, window, cx| devdeck_ui::pop_modal(window, cx));
        let _ = self.main.update(cx, |_, window, cx| crate::cheat_sheet::open(window, cx));
        self.frame(cx).await;
        let _ = self.main.update(cx, |_, window, cx| devdeck_ui::pop_modal(window, cx));
        let _ = self.main.update(cx, |_, window, cx| devdeck_ui::open_menu(Default::default(), vec![], window, cx));
        self.frame(cx).await;
        let main = self.main;
        cx.update(|cx| devdeck_ui::overlay::layer(main, cx).update(cx, |l, cx| {
            l.menu = None;
            cx.notify();
        }));
    }

    /// Memory and engine counters: views and overlay layers must stay flat, no view may
    /// outlive its tab, no metrics sampler or pod interest may outlive its view.
    fn report(&self, n: u64, cx: &mut AsyncApp) {
        let m = stats::memory();
        let (views, leaked, windows, layers, samplers, interests) = cx.update(|cx| {
            let s = self.shell.read(cx);
            let decks: Vec<_> = s.workspaces.iter().map(|w| w.deck.clone()).collect();
            let clusters: Vec<_> = s.workspaces.iter().filter_map(|w| w.services.read(cx).cluster.clone()).collect();
            let samplers: usize = clusters.iter().map(|c| c.read(cx).samplers()).sum();
            let interests: usize = clusters.iter().map(|c| c.read(cx).pod_interests()).sum();
            let views: usize = decks.iter().map(|d| d.read(cx).live_views()).sum();
            let leaked: usize = decks.iter().map(|d| d.update(cx, |d, _| d.leaked_views(3))).sum();
            (views, leaked, cx.windows().len(), devdeck_ui::overlay::layer_count(cx), samplers, interests)
        });
        eprintln!(
            "devdeck-stress cycle={n} rss_kb={} pss_kb={} uss_kb={} views={views} leaked_views={leaked} windows={windows} layers={layers} samplers={samplers} pod_interests={interests}",
            m.rss, m.pss, m.uss
        );
        // Between cycles nothing is open: no sampler, no pod interest, no view outliving
        // its tab.
        if samplers != 0 || interests != 0 || leaked != 0 {
            eprintln!("devdeck-stress cycle={n} step failed: {samplers} samplers, {interests} pod interests and {leaked} views left");
        }
    }
}
