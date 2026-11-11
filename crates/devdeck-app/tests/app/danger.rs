//! Dangerous actions (R5.6): each runs only once confirmed in its modal, which opens on
//! Cancel so a stray Enter cancels. The token is checked by the compiler for deploys,
//! stops and note deletes; git's dangerous ops are checked when they run. The other
//! modals' Enter is tested with their views (git_panel, ports, notes).

use devdeck_app::cluster::{Cluster, LOAD_DELAY, ROLLOUT_STEP};
use devdeck_app::repo::Repo;
use devdeck_app::shell::Shell;
use devdeck_app::views::service_view::ServiceView;
use devdeck_core::cluster::RolloutState;
use devdeck_core::git::{Doing, Op, PushPlan};
use devdeck_core::view_addr::ViewAddr;
use gpui::{AppContext, Entity, TestAppContext};
use std::time::{Duration, Instant};

fn start(cx: &mut TestAppContext) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir("danger", cx));
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
fn a_deploy_asks_on_cancel_and_goes_from_its_button(cx: &mut TestAppContext) {
    let shell = start(cx);
    let c = cluster(&shell, cx);
    let view = service(&shell, cx);
    let main = cx.read(|cx| shell.read(cx).main_window().expect("main"));
    let layers = |cx: &mut TestAppContext| cx.update(|cx| devdeck_ui::overlay::layer_state(main.into(), cx).1);
    // Rendered once so the view knows its window (modals open there).
    cx.update_window(main.into(), |_, window, _| window.refresh()).unwrap();
    cx.run_until_parked();
    cx.update(|cx| view.update(cx, |v, cx| v.tag.update(cx, |i, cx| i.set_text("lan-160", cx))));
    assert_eq!(cx.read(|cx| view.read(cx).save_blocked(cx)), None, "no switch to turn on first");
    let ask = |cx: &mut TestAppContext| {
        cx.update(|cx| view.update(cx, |v, cx| v.save(cx)));
        cx.run_until_parked();
    };
    ask(cx);
    assert_eq!(layers(cx), 1, "the deploy confirmation");
    cx.simulate_keystrokes(main.into(), "enter");
    cx.run_until_parked();
    assert_eq!(layers(cx), 0, "a stray Enter cancels");
    assert!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").is_none()), "nothing deployed");
    ask(cx);
    cx.simulate_keystrokes(main.into(), "tab enter");
    cx.executor().advance_clock(ROLLOUT_STEP * 3);
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| c.read(cx).rollout("dev35", "entityservice").map(|r| r.state.clone())), Some(RolloutState::Done), "Tab to Deploy, Enter");
}

/// A real repository (no origin needed): a dangerous op is refused without its token,
/// before anything else; the others reach the usual checks.
#[gpui::test]
fn a_replace_push_and_an_abort_run_only_confirmed(cx: &mut TestAppContext) {
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
    let run = |cx: &mut TestAppContext, op: Op| cx.update(|cx| repo.update(cx, |r, cx| r.run(op, "Op".into(), "doing…".into(), cx)));
    let unconfirmed = Err("it needs confirming first".to_string());
    assert_eq!(run(cx, replace.clone()), unconfirmed);
    assert_eq!(run(cx, Op::Abort(Doing::Rebasing)), unconfirmed);
    assert_eq!(run(cx, Op::Abort(Doing::Bisecting)), Err("nothing to abort".into()), "a bisect's abort resets nothing: the usual checks");
    cx.read(|cx| {
        let r = repo.read(cx);
        assert_eq!(r.refusal(&replace), None, "nothing to turn on first: its button is live");
        assert_eq!(r.refusal(&Op::Abort(Doing::Rebasing)), Some("nothing to abort".into()));
    });
}
