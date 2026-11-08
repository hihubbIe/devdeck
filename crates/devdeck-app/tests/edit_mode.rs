//! M3 step 3b: edit mode (R5.6). Off at start; destructive actions refused with the reason
//! while it's off, below the UI (a confirmation opened while on and confirmed after
//! turning it off too); the title bar button turns it on after a modal (focus on Cancel)
//! and off without one.

use devdeck_app::cluster::{Cluster, LOAD_DELAY, ROLLOUT_STEP};
use devdeck_app::edit_mode::{self, OFF};
use devdeck_app::repo::Repo;
use devdeck_app::shell::Shell;
use devdeck_app::views::service_view::ServiceView;
use devdeck_core::cluster::Rollout;
use devdeck_core::git::{Doing, Op, PushPlan};
use devdeck_core::view_addr::ViewAddr;
use gpui::{AppContext, Entity, TestAppContext};
use std::time::{Duration, Instant};

fn start(cx: &mut TestAppContext) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir("edit-mode", cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn cluster(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Cluster> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.read(cx).cluster.clone().unwrap())
}

fn service(shell: &Entity<Shell>, cx: &mut TestAppContext) -> Entity<ServiceView> {
    let deck = cx.read(|cx| shell.read(cx).active_deck());
    let r = ViewAddr::Svc { env: "dev35".into(), name: "entityservice".into() }.to_ref();
    let open = r.clone();
    cx.update(|cx| deck.update(cx, |d, cx| d.apply(cx, |l| l.open(open, l.main().focused).map(|_| ()))));
    cx.executor().advance_clock(LOAD_DELAY);
    cx.run_until_parked();
    cx.read(|cx| deck.read(cx).view(&r).unwrap().downcast::<ServiceView>().unwrap())
}

#[gpui::test]
fn deploys_wait_for_edit_mode_turned_on_through_its_modal(cx: &mut TestAppContext) {
    let shell = start(cx);
    assert!(!cx.read(edit_mode::is_on), "off at start");
    let c = cluster(&shell, cx);
    let view = service(&shell, cx);
    let deploy = |cx: &mut TestAppContext| cx.update(|cx| c.update(cx, |c, cx| c.set_image("dev35", "entityservice", "lan-177", false, cx)));
    assert_eq!(deploy(cx), Err(OFF.into()), "refused below the UI");
    assert_eq!(cx.read(|cx| view.read(cx).save_blocked(cx)), Some(OFF.into()), "Save says why before asking anything");

    // The title bar button: a modal first, focus on Cancel; Tab then Enter turns it on.
    let main = cx.read(|cx| shell.read(cx).main_window().expect("main"));
    let layers = |cx: &mut TestAppContext| cx.update(|cx| devdeck_ui::overlay::layer_state(main.into(), cx).1);
    let toggle = |cx: &mut TestAppContext| {
        cx.update_window(main.into(), |_, window, cx| edit_mode::toggle(window, cx)).unwrap();
        cx.run_until_parked();
    };
    toggle(cx);
    assert_eq!(layers(cx), 1, "asked first");
    cx.simulate_keystrokes(main.into(), "enter");
    assert!(!cx.read(edit_mode::is_on) && layers(cx) == 0, "a stray Enter cancels");
    toggle(cx);
    cx.simulate_keystrokes(main.into(), "tab enter");
    assert!(cx.read(edit_mode::is_on) && layers(cx) == 0);
    assert_eq!(deploy(cx), Ok(()));
    cx.executor().advance_clock(ROLLOUT_STEP * 3);
    cx.run_until_parked();
    assert!(matches!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").cloned()), Some(Rollout::Done { .. })));
    // Off at once, no modal; the done rollout's undo is refused then.
    toggle(cx);
    assert!(!cx.read(edit_mode::is_on) && layers(cx) == 0);
    assert_eq!(cx.read(|cx| view.read(cx).undo_blocked(cx)), Some(OFF.into()), "the undo button says why");
    assert_eq!(cx.update(|cx| c.update(cx, |c, cx| c.undo("dev35", "entityservice", false, cx))), Err(OFF.into()));
}

#[gpui::test]
fn a_deploy_confirmed_after_edit_mode_went_off_is_refused(cx: &mut TestAppContext) {
    let shell = start(cx);
    let c = cluster(&shell, cx);
    let view = service(&shell, cx);
    let main = cx.read(|cx| shell.read(cx).main_window().expect("main"));
    cx.update(|cx| edit_mode::set(true, cx));
    // Rendered once so the view knows its window (modals open there).
    cx.update_window(main.into(), |_, window, _| window.refresh()).unwrap();
    cx.run_until_parked();
    cx.update(|cx| view.update(cx, |v, cx| v.tag.update(cx, |i, cx| i.set_text("lan-160", cx))));
    cx.update(|cx| view.update(cx, |v, cx| v.save(cx)));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| devdeck_ui::overlay::layer_state(main.into(), cx).1), 1, "the deploy confirmation");
    cx.update(|cx| edit_mode::set(false, cx));
    cx.simulate_keystrokes(main.into(), "enter");
    cx.run_until_parked();
    assert!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").is_none()), "nothing deployed");
}

/// A real repository (no origin needed): refusals are about edit mode, not "not read yet".
#[gpui::test]
fn a_replace_push_and_an_abort_wait_for_edit_mode(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let _shell = start(cx);
    let root = cx.update(|cx| devdeck_app::test_dir("repo", cx));
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git").arg("-C").arg(&root).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
        assert!(ok.status.success(), "{args:?}");
    };
    git(&["init", "-q", "-b", "dev"]);
    git(&["-c", "user.name=a", "-c", "user.email=a@x", "commit", "-q", "--allow-empty", "-m", "one"]);
    let repo = cx.update(|cx| cx.new(|cx| Repo::new(root.clone(), None, Vec::new(), cx)));
    let wait = |cx: &mut TestAppContext, what: &str, done: &dyn Fn(&TestAppContext) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(cx) {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(20));
            cx.run_until_parked();
        }
    };
    wait(cx, "the repository opens", &|cx| !cx.read(|cx| repo.read(cx).opening()));
    let _looking = cx.update(|cx| repo.update(cx, |r, cx| r.look(cx)));
    wait(cx, "the repository read", &|cx| cx.read(|cx| repo.read(cx).full().is_some()));
    let replace = Op::Push(PushPlan::Replace { remote: "origin".into(), branch: "dev".into(), expected: String::new(), head: String::new(), replaced: vec![] });
    let send = Op::Push(PushPlan::Send { remote: "origin".into(), branch: "dev".into(), head: String::new(), commits: 1 });
    cx.read(|cx| {
        let r = repo.read(cx);
        assert_eq!(r.refusal(&replace, cx), Some(OFF.into()));
        assert_eq!(r.refusal(&Op::Abort(Doing::Rebasing), cx), Some(OFF.into()));
        assert_eq!(r.refusal(&send, cx), None, "a plain push isn't gated");
        assert_eq!(r.refusal(&Op::Abort(Doing::Bisecting), cx), Some("nothing to abort".into()), "a bisect's abort isn't gated (and nothing's under way)");
    });
    cx.update(|cx| edit_mode::set(true, cx));
    cx.read(|cx| {
        let r = repo.read(cx);
        assert_eq!(r.refusal(&replace, cx), None);
        assert_eq!(r.refusal(&Op::Abort(Doing::Rebasing), cx), Some("nothing to abort".into()), "past edit mode: nothing under way");
    });
}
