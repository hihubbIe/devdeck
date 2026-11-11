//! M1 step 5b: service inspector on the fake cluster (R7.2–R7.6, R7.8): tag edit →
//! confirm → rollout → ready, broken tag → failing → fixed by another deploy, protected unlock, metrics
//! sampled only while visible. Fake clock only.

use devdeck_app::cluster::{Cluster, LOAD_DELAY, METRICS_TICK, ROLLOUT_STEP};
use devdeck_app::shell::Shell;
use devdeck_app::views::service_view::ServiceView;
use devdeck_core::cluster::RolloutState;
use devdeck_core::layout::ViewRef;
use devdeck_core::view_addr::ViewAddr;
use devdeck_ui::overlay::layer_state;
use gpui::{AnyWindowHandle, AppContext, Entity, Focusable, TestAppContext};
use std::time::Duration;

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(tag, cx));
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
    cx.simulate_keystrokes(main, "tab enter");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 0);
    assert!(matches!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").map(|r| r.state.clone())), Some(RolloutState::Rolling { .. })));
    advance(cx, ROLLOUT_STEP * before.replicas);
    let after = cx.read(|cx| c.read(cx).deployment("dev35", "entityservice").unwrap().clone());
    assert_eq!(after.tag(), "lan-160");
    assert!(!cx.read(|cx| view.read(cx).dirty(cx)), "the editor now shows the deployed tag");
    let history = cx.read(|cx| c.read(cx).history("dev35", "entityservice").to_vec());
    assert_eq!((history.len(), history[0].from.as_str(), history[0].by.as_str(), history[0].outcome.as_str()), (1, before.tag().as_str(), "demo via Kugelblitz", "done"));
    let suggestions = cx.read(|cx| c.read(cx).tag_suggestions("dev35", "entityservice"));
    assert!(suggestions.iter().any(|(t, why)| *t == before.tag() && why == "previous"), "the previous tag is suggested");
    assert_eq!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").map(|r| r.state.clone())), Some(RolloutState::Done));
}

#[gpui::test]
fn a_broken_tag_keeps_failing_until_another_deploy_replaces_it(cx: &mut TestAppContext) {
    let shell = start(cx, "broken");
    let main = main_window(&shell, cx);
    let c = cluster(&shell, cx);
    let _view = open_service(&shell, cx, "dev35", "graphservice");
    let tag = cx.read(|cx| c.read(cx).deployment("dev35", "graphservice").unwrap().tag());
    type_tag(main, cx, "lan-broken");
    cx.simulate_keystrokes(main, "ctrl-s tab enter");
    advance(cx, ROLLOUT_STEP * 4);
    let state = cx.read(|cx| c.read(cx).rollout("dev35", "graphservice").map(|r| r.state.clone()));
    let Some(RolloutState::Failing { reason, .. }) = state else { panic!("failing: {state:?}") };
    assert!(reason.starts_with("image ") && reason.ends_with(":lan-broken not found"), "{reason}");
    assert!(cx.read(|cx| c.read(cx).pods("dev35", "graphservice").iter().any(|p| p.tag == "lan-broken" && !p.ready)), "a pod stuck pulling");
    // The fix: another deploy, right away (nothing waits for the failing one).
    type_tag(main, cx, &tag);
    cx.simulate_keystrokes(main, "ctrl-s tab enter");
    let replicas = cx.read(|cx| c.read(cx).deployment("dev35", "graphservice").unwrap().replicas);
    advance(cx, ROLLOUT_STEP * replicas);
    let history = cx.read(|cx| c.read(cx).history("dev35", "graphservice").to_vec());
    assert_eq!(history.iter().map(|r| r.outcome.as_str()).collect::<Vec<_>>(), [format!("replaced by {tag}").as_str(), "done"]);
    assert!(cx.read(|cx| c.read(cx).pods("dev35", "graphservice").iter().all(|p| p.tag == tag)));
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
    cx.simulate_keystrokes(main, "tab enter");
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
    assert!(cx.read(|cx| matches!(c.read(cx).peek("dev36"), devdeck_core::cluster::Remote::Error(_))), "dev36 is unreachable");
    let missing = devdeck_core::demo::catalog("work").cluster.unwrap().services.into_iter().find(|s| devdeck_core::demo::deployment("dls", s).is_none()).unwrap();
    let _ = open_service(&shell, cx, "dls", &missing);
    assert!(cx.read(|cx| c.read(cx).deployment("dls", &missing).is_none()));
    assert!(cx.read(|cx| c.read(cx).metrics_allowed("lab")).is_err());
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let unknown = ViewRef::new("svc/nowhere/x");
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(unknown.clone(), l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    assert!(cx.read(|cx| deck.read(cx).view(&unknown)).is_some(), "an unknown env gets a message view");
}

#[gpui::test]
fn esc_in_an_input_returns_to_its_view_in_any_window(cx: &mut TestAppContext) {
    let shell = start(cx, "esc");
    let main = main_window(&shell, cx);
    let view = open_service(&shell, cx, "dev35", "entityservice");
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let svc = ViewAddr::Svc { env: "dev35".into(), name: "entityservice".into() }.to_ref();
    // In a detached window.
    let r = svc.clone();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| {
        let (p, i) = l.find_view(&r).unwrap();
        l.detach(p, i).map(|_| ())
    })));
    cx.run_until_parked();
    let detached = cx.windows().into_iter().find(|w| *w != main).expect("detached window");
    let focus = cx.read(|cx| view.read(cx).focus_handle(cx));
    cx.update_window(detached, |_, window, cx| focus.focus(window, cx)).unwrap();
    cx.simulate_keystrokes(detached, "t");
    let tag = cx.read(|cx| view.read(cx).tag.read(cx).focus_handle(cx));
    assert!(cx.update_window(detached, |_, window, _| tag.is_focused(window)).unwrap(), "t edits the tag");
    cx.simulate_keystrokes(detached, "escape");
    assert!(cx.update_window(detached, |_, window, _| focus.is_focused(window)).unwrap(), "Esc gives the keys back to the view");
}

#[gpui::test]
fn esc_in_a_modal_input_closes_the_modal_and_esc_in_a_view_without_focus_goes_to_the_panes(cx: &mut TestAppContext) {
    let shell = start(cx, "esc-modal");
    let main = main_window(&shell, cx);
    let view = open_service(&shell, cx, "hades", "proxyapi");
    type_tag(main, cx, "lan-1");
    cx.simulate_keystrokes(main, "ctrl-s");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 1, "unlock modal with its input focused");
    cx.simulate_keystrokes(main, "escape");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 0, "Esc closes the modal");
    let tag = cx.read(|cx| view.read(cx).tag.read(cx).focus_handle(cx));
    assert!(cx.update_window(main, |_, window, _| tag.is_focused(window)).unwrap(), "focus back where it was, not taken by the view");
    // The gallery has inputs but no focus handle of its own: Esc goes to the pane area.
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(devdeck_core::layout::ViewRef::new("demo/gallery"), l.main().focused).map(|_| ()))));
    cx.run_until_parked();
    let mut vcx = gpui::VisualTestContext::from_window(main, cx);
    let input = vcx.debug_bounds("empty-input").expect("gallery input");
    vcx.simulate_click(input.center(), gpui::Modifiers::none());
    vcx.simulate_keystrokes("escape");
    let panes = vcx.read(|cx| deck.read(cx).layout().panes_in(devdeck_core::layout::MAIN_WINDOW).unwrap().len());
    vcx.simulate_keystrokes("ctrl-\\");
    vcx.run_until_parked();
    assert_eq!(vcx.read(|cx| deck.read(cx).layout().panes_in(devdeck_core::layout::MAIN_WINDOW).unwrap().len()), panes + 1, "pane keys work");
}

#[gpui::test]
fn r_redeploys_the_same_branch_and_the_fake_rolls_new_pods(cx: &mut TestAppContext) {
    let shell = start(cx, "redeploy");
    let main = main_window(&shell, cx);
    let c = cluster(&shell, cx);
    let _view = open_service(&shell, cx, "dev35", "entityservice");
    let before = cx.read(|cx| c.read(cx).deployment("dev35", "entityservice").unwrap().clone());
    let old_pods: Vec<String> = cx.read(|cx| c.read(cx).pods("dev35", "entityservice").iter().map(|p| p.name.clone()).collect());
    assert!(!old_pods.is_empty());
    cx.simulate_keystrokes(main, "r");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 1, "a confirmation, not a redeploy");
    assert!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").is_none()));
    cx.simulate_keystrokes(main, "tab enter");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 0, "closed once the cluster took it");
    let r = cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").cloned()).expect("followed");
    assert!(r.redeploy() && r.by == "demo via Kugelblitz" && matches!(r.state, RolloutState::Rolling { .. }), "{r:?}");
    advance(cx, ROLLOUT_STEP * before.replicas);
    let after = cx.read(|cx| c.read(cx).deployment("dev35", "entityservice").unwrap().clone());
    assert_eq!((after.tag(), after.revision), (before.tag(), before.revision + 1), "the same tag, a new revision");
    assert!(after.restarted_at.is_some());
    assert_eq!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").map(|r| r.state.clone())), Some(RolloutState::Done));
    let pods = cx.read(|cx| c.read(cx).pods("dev35", "entityservice").to_vec());
    assert!(pods.iter().all(|p| !old_pods.contains(&p.name)), "every pod replaced");
    let history = cx.read(|cx| c.read(cx).history("dev35", "entityservice").to_vec());
    assert_eq!((history.len(), history[0].redeploy(), history[0].by.as_str(), history[0].outcome.as_str()), (1, true, "demo via Kugelblitz", "done"));
}

#[gpui::test]
fn a_redeploy_confirmed_after_someone_deployed_is_refused_in_its_confirmation(cx: &mut TestAppContext) {
    let shell = start(cx, "redeploy-changed");
    let main = main_window(&shell, cx);
    let c = cluster(&shell, cx);
    let _view = open_service(&shell, cx, "dev35", "entityservice");
    cx.simulate_keystrokes(main, "r");
    cx.run_until_parked();
    // Someone deploys meanwhile.
    cx.update(|cx| {
        c.update(cx, |c, cx| {
            let mut list: Vec<_> = c.peek("dev35").ready().map(|m| m.values().cloned().collect()).unwrap_or_default();
            if let Some(d) = list.iter_mut().find(|d| d.name == "entityservice") {
                d.main_mut().image = devdeck_core::cluster::Image::parse("registry/entityservice:hotfix");
                d.revision += 1;
            }
            c.apply("dev35", devdeck_app::cluster::EnvUpdate::Deployments(list), cx);
        })
    });
    cx.simulate_keystrokes(main, "tab enter");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 1, "the confirmation stays, saying why");
    let mut vcx = gpui::VisualTestContext::from_window(main, cx);
    assert!(vcx.debug_bounds("deploy-error").is_some(), "the refusal is shown in it");
}

/// The fake refuses a write pinned to an image that changed meanwhile (its whole image,
/// as written): the confirmation stays and says why, and nothing is deployed.
#[gpui::test]
fn a_deploy_confirmed_after_someone_deployed_is_refused_in_its_confirmation(cx: &mut TestAppContext) {
    let shell = start(cx, "deploy-changed");
    let main = main_window(&shell, cx);
    let c = cluster(&shell, cx);
    let _view = open_service(&shell, cx, "dev35", "entityservice");
    type_tag(main, cx, "lan-160");
    cx.simulate_keystrokes(main, "ctrl-s");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 1);
    let hotfix = cx.update(|cx| {
        c.update(cx, |c, cx| {
            let mut list: Vec<_> = c.peek("dev35").ready().map(|m| m.values().cloned().collect()).unwrap_or_default();
            let d = list.iter_mut().find(|d| d.name == "entityservice").unwrap();
            let image = format!("{}:hotfix-from-slack", d.main().image.repo);
            d.main_mut().image = devdeck_core::cluster::Image::parse(&image);
            d.revision += 1;
            c.apply("dev35", devdeck_app::cluster::EnvUpdate::Deployments(list), cx);
            image
        })
    });
    cx.simulate_keystrokes(main, "tab enter");
    advance(cx, ROLLOUT_STEP * 3);
    assert_eq!(modals(main, cx), 1, "the confirmation stays, saying why");
    let modal = cx.update(|cx| devdeck_ui::overlay::layer(main, cx).read(cx).modals.last().cloned()).unwrap();
    let confirm = modal.downcast::<devdeck_app::views::service_view::ConfirmDeploy>().unwrap();
    assert_eq!(cx.read(|cx| confirm.read(cx).error().map(String::from)), Some("entityservice changed since you confirmed".into()));
    assert_eq!(cx.read(|cx| c.read(cx).deployment("dev35", "entityservice").unwrap().main().image.raw.clone()), hotfix, "nothing deployed over it");
}

#[gpui::test]
fn the_redeploy_button_says_why_not_and_a_protected_env_asks_its_name_first(cx: &mut TestAppContext) {
    let shell = start(cx, "redeploy-guards");
    let main = main_window(&shell, cx);
    assert_eq!(devdeck_app::registry::by_id("svc.redeploy").and_then(|s| s.key_text()).as_deref(), Some("r"), "registered, on r");
    let view = open_service(&shell, cx, "hades", "proxyapi");
    assert_eq!(cx.read(|cx| view.read(cx).redeploy_blocked(cx)), Some("hades is protected: unlock it first".into()));
    cx.simulate_keystrokes(main, "r");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 1, "unlock asked first");
    cx.simulate_input(main, "hades");
    cx.simulate_keystrokes(main, "enter");
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| view.read(cx).redeploy_blocked(cx)), None);
    cx.simulate_keystrokes(main, "r");
    cx.run_until_parked();
    assert_eq!(modals(main, cx), 1, "now the confirmation");
}
