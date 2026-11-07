//! M1 step 5b: service inspector on the fake cluster (R7.2–R7.6, R7.8): tag edit →
//! confirm → rollout → ready, broken tag → failure → undo, protected unlock, metrics
//! sampled only while visible. Fake clock only.

use devdeck_app::cluster::{Cluster, LOAD_DELAY, METRICS_TICK, ROLLOUT_STEP};
use devdeck_app::shell::Shell;
use devdeck_app::views::service_view::ServiceView;
use devdeck_core::cluster::Rollout;
use devdeck_core::layout::ViewRef;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::overlay::layer_state;
use gpui::{AnyWindowHandle, Entity, TestAppContext};
use std::time::Duration;

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = std::env::temp_dir().join(format!("devdeck-svctest-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&config);
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().expect("main").into())
}

fn advance(cx: &mut TestAppContext, d: Duration) {
    cx.executor().advance_clock(d);
    cx.run_until_parked();
}

fn cluster(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Cluster> {
    cx.read(|cx| {
        let s = shell.read(cx);
        let w = s.workspaces.iter().find(|w| w.id == "work").unwrap();
        w.services.read(cx).cluster.clone().unwrap()
    })
}

/// Opens a service view in the focused pane (it gets the keys) and waits for the env.
fn open_service(shell: &Entity<Shell>, cx: &mut TestAppContext, env: &str, name: &str) -> Entity<ServiceView> {
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = ViewAddr::Svc { env: env.into(), name: name.into() }.to_ref();
    let open = r.clone();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(open, l.main().focused).map(|_| ()))));
    advance(cx, LOAD_DELAY);
    cx.read(|cx| deck.read(cx).view(&r).unwrap().downcast::<ServiceView>().unwrap())
}

fn modals(main: AnyWindowHandle, cx: &mut TestAppContext) -> usize {
    cx.update(|cx| layer_state(main, cx).1)
}

fn type_tag(main: AnyWindowHandle, cx: &mut TestAppContext, tag: &str) {
    cx.simulate_keystrokes(main, "t ctrl-a");
    cx.simulate_input(main, tag);
    cx.run_until_parked();
}

#[gpui::test]
fn tag_edit_confirm_rollout_ready(cx: &mut TestAppContext) {
    let shell = start(cx, "deploy");
    let main = main_window(&shell, cx);
    let c = cluster(&shell, cx);
    let view = open_service(&shell, cx, "dev35", "entityservice");
    let before = cx.read(|cx| c.read(cx).deployment("dev35", "entityservice").unwrap().clone());
    assert!(!cx.read(|cx| view.read(cx).dirty(cx)));
    type_tag(main, cx, "lan-160");
    assert!(cx.read(|cx| view.read(cx).dirty(cx)), "unsaved marker");
    cx.simulate_keystrokes(main, "ctrl-s");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 1, "a confirmation, not a deploy");
    assert!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").is_none()));
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 0);
    assert!(matches!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").cloned()), Some(Rollout::Progressing { .. })));
    assert_eq!(cx.read(|cx| view.read(cx).save_blocked(cx)).as_deref(), Some("a rollout is in progress"));
    advance(cx, ROLLOUT_STEP * before.replicas);
    let after = cx.read(|cx| c.read(cx).deployment("dev35", "entityservice").unwrap().clone());
    assert_eq!(after.tag, "lan-160");
    assert!(!cx.read(|cx| view.read(cx).dirty(cx)), "the editor now shows the deployed tag");
    let history = cx.read(|cx| c.read(cx).history("dev35", "entityservice").to_vec());
    assert_eq!((history.len(), history[0].from.as_str(), history[0].outcome.as_str()), (1, before.tag.as_str(), "ready"));
    let suggestions = cx.read(|cx| c.read(cx).tag_suggestions("dev35", "entityservice"));
    assert!(suggestions.iter().any(|(t, why)| *t == before.tag && why == "previous"), "the previous tag is suggested");
    // Typing u in the editor writes a u; it doesn't undo.
    cx.simulate_keystrokes(main, "t");
    cx.simulate_input(main, "u");
    assert!(matches!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").cloned()), Some(Rollout::Done { .. })));
}

#[gpui::test]
fn broken_tag_fails_and_undo_restores(cx: &mut TestAppContext) {
    let shell = start(cx, "broken");
    let main = main_window(&shell, cx);
    let c = cluster(&shell, cx);
    let view = open_service(&shell, cx, "dev35", "graphservice");
    let tag = cx.read(|cx| c.read(cx).deployment("dev35", "graphservice").unwrap().tag.clone());
    type_tag(main, cx, "lan-broken");
    cx.simulate_keystrokes(main, "ctrl-s enter");
    advance(cx, ROLLOUT_STEP);
    let Some(Rollout::Failed { reason, .. }) = cx.read(|cx| c.read(cx).rollout("dev35", "graphservice").cloned()) else { panic!("failed") };
    assert!(reason.starts_with("ImagePullBackOff"));
    assert!(cx.read(|cx| c.read(cx).pods("dev35", "graphservice").iter().any(|p| p.tag == "lan-broken" && !p.ready)), "a pod stuck pulling");
    assert_eq!(cx.read(|cx| c.read(cx).deployment("dev35", "graphservice").unwrap().tag.clone()), tag, "still on the old tag");
    // Esc leaves the editor (and reverts it), then u undoes.
    cx.simulate_keystrokes(main, "escape u");
    cx.run_until_parked();
    assert!(cx.read(|cx| c.read(cx).rollout("dev35", "graphservice").is_none()), "undo clears the failure");
    assert!(cx.read(|cx| c.read(cx).pods("dev35", "graphservice").iter().all(|p| p.tag == tag)));
    assert!(!cx.read(|cx| view.read(cx).dirty(cx)), "and reverts the editor");
}

#[gpui::test]
fn protected_envs_need_the_typed_name(cx: &mut TestAppContext) {
    let shell = start(cx, "protected");
    let main = main_window(&shell, cx);
    let c = cluster(&shell, cx);
    let view = open_service(&shell, cx, "hades", "proxyapi");
    type_tag(main, cx, "lan-161");
    cx.simulate_keystrokes(main, "ctrl-s");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 1, "unlock asked first");
    cx.simulate_input(main, "Hades");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert!(!cx.read(|cx| view.read(cx).unlocked), "wrong case refused");
    assert_eq!(modals(main, cx), 1);
    cx.simulate_keystrokes(main, "ctrl-a");
    cx.simulate_input(main, "hades");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert!(cx.read(|cx| view.read(cx).unlocked));
    assert_eq!(modals(main, cx), 0);
    cx.simulate_keystrokes(main, "ctrl-s");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 1, "now the confirmation");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert!(cx.read(|cx| c.read(cx).rollout("hades", "proxyapi").is_some()));
}

#[gpui::test]
fn metrics_are_sampled_only_while_visible(cx: &mut TestAppContext) {
    let shell = start(cx, "sampling");
    let c = cluster(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let view = open_service(&shell, cx, "dev35", "entityservice");
    assert!(cx.read(|cx| view.read(cx).is_sampling() && c.read(cx).is_sampling()));
    advance(cx, METRICS_TICK * 2);
    let ticks = cx.read(|cx| c.read(cx).metric_ticks);
    assert!(ticks >= 2);
    // Another tab in front: hidden.
    let svc = ViewAddr::Svc { env: "dev35".into(), name: "entityservice".into() }.to_ref();
    cx.update(|cx| {
        deck.update(cx, |d, cx| {
            d.apply(cx, |l| {
                let (p, _) = l.find_view(&svc).unwrap();
                l.activate(p, 0)
            });
        })
    });
    cx.run_until_parked();
    assert!(!cx.read(|cx| view.read(cx).is_sampling()));
    advance(cx, METRICS_TICK);
    assert!(!cx.read(|cx| c.read(cx).is_sampling()), "the cluster stops ticking");
    let stopped = cx.read(|cx| c.read(cx).metric_ticks);
    advance(cx, METRICS_TICK * 3);
    assert_eq!(cx.read(|cx| c.read(cx).metric_ticks), stopped);
    // Back in front, then the workspace is hidden.
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(svc.clone(), l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    assert!(cx.read(|cx| view.read(cx).is_sampling()));
    let personal = cx.read(|cx| shell.read(cx).workspaces.iter().position(|w| w.id == "personal").unwrap());
    cx.update(|cx| shell.update(cx, |s, cx| s.switch(personal, cx)));
    cx.run_until_parked();
    assert!(!cx.read(|cx| view.read(cx).is_sampling()), "hidden workspace: no sampling");
    advance(cx, METRICS_TICK);
    assert!(!cx.read(|cx| c.read(cx).is_sampling()));
}

#[gpui::test]
fn closing_the_view_stops_sampling_and_frees_it(cx: &mut TestAppContext) {
    let shell = start(cx, "close");
    let c = cluster(&shell, cx);
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let _ = open_service(&shell, cx, "dev35", "entityservice");
    let svc = ViewAddr::Svc { env: "dev35".into(), name: "entityservice".into() }.to_ref();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.close_view(&svc))));
    advance(cx, METRICS_TICK);
    assert!(!cx.read(|cx| c.read(cx).is_sampling()));
    for _ in 0..3 {
        cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |_| Ok(()))));
        cx.run_until_parked();
    }
    assert_eq!(cx.update(|cx| deck.update(cx, |d, _| d.leaked_views(3))), 0, "the view is released");
}

#[gpui::test]
fn unreachable_missing_and_forbidden_states(cx: &mut TestAppContext) {
    let shell = start(cx, "states");
    let c = cluster(&shell, cx);
    let _ = open_service(&shell, cx, "dev36", "entityservice");
    let err = cx.read(|cx| c.read(cx).peek("dev36"));
    assert!(matches!(err, devdeck_core::cluster::Remote::Error(_)), "dev36 is unreachable");
    let missing = cx.read(|cx| c.read(cx).spec.services.iter().find(|s| devdeck_core::demo::deployment("dls", s).is_none()).cloned()).unwrap();
    let _ = open_service(&shell, cx, "dls", &missing);
    assert!(cx.read(|cx| c.read(cx).deployment("dls", &missing).is_none()));
    assert!(cx.read(|cx| c.read(cx).metrics_allowed("lab")).is_err());
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let unknown = ViewRef::new("svc/nowhere/x");
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(unknown.clone(), l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).view(&unknown)).is_some(), "an unknown env gets a message view");
}
