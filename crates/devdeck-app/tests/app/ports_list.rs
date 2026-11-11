//! The Ports list (docs/plans/ports-view.md, 5a): ports held by a run, a terminal's
//! program or a holder the catalog names, the ports the workspace knows even free, and
//! with "machine" every other listener. 5b: Ports as a full view, a port's page by kind,
//! its actions, its HTTP answer. The scan is set by hand (the demo has none).

use devdeck_app::port_scan::{PortHolder, PortScan, ProcInfo};
use devdeck_app::runner::RunKey;
use devdeck_app::services::Services;
use devdeck_app::shell::Shell;
use devdeck_app::sidebar::Activity;
use devdeck_app::views::port_view::{http_probe, status_line};
use devdeck_core::layout::ViewRef;
use devdeck_core::view_addr::ViewAddr;
use gpui::{AnyWindowHandle, Entity, Modifiers, TestAppContext, VisualTestContext};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};

fn start(cx: &mut TestAppContext, tag: &str) -> Entity<Shell> {
    let config = cx.update(|cx| devdeck_app::test_dir(&format!("{tag}-config"), cx));
    cx.update(|cx| devdeck_app::init_with_config_dir(&config, cx));
    let shell = cx.update(|cx| Shell::start(devdeck_app::demo_workspaces(), "work", None, cx));
    cx.update(|cx| shell.update(cx, |s, _| s.quit_for_real = false));
    cx.run_until_parked();
    shell
}

fn services(shell: &Entity<Shell>, cx: &TestAppContext) -> Entity<Services> {
    cx.read(|cx| shell.read(cx).workspaces.iter().find(|w| w.id == "work").unwrap().services.clone())
}

/// A scan: a program in terminal 3 on :8000 (every interface), and Postgres elsewhere on
/// the machine.
fn scanned(s: &Entity<Services>, cx: &mut TestAppContext) {
    let ports = cx.read(|cx| s.read(cx).ports.clone());
    cx.update(|cx| {
        ports.update(cx, |p, cx| {
            p.scan = PortScan {
                listening: BTreeMap::from([(RunKey::terminal(3), vec![8000])]),
                held: BTreeMap::new(),
                addrs: BTreeMap::from([(8000, IpAddr::V4(Ipv4Addr::UNSPECIFIED)), (5432, IpAddr::V4(Ipv4Addr::LOCALHOST))]),
                machine: BTreeMap::from([(5432, PortHolder::Process { pid: 77, command: "postgres -D /var/lib/pg".into() })]),
                procs: BTreeMap::from([(8000, ProcInfo { pid: 4242, command: "python3 -m http.server".into(), started: Some(1_762_500_000) })]),
            };
            cx.notify();
        })
    });
}

#[gpui::test]
fn the_list_has_what_listens_and_the_known_ports_free(cx: &mut TestAppContext) {
    let shell = start(cx, "ports-rows");
    let s = services(&shell, cx);
    scanned(&s, cx);
    let rows = cx.read(|cx| s.read(cx).port_rows(false, cx));
    // A terminal's program: its own process holds the port, the terminal is said.
    let web = rows.iter().find(|r| r.port == 8000).expect("the terminal's port");
    let held = web.held.as_ref().unwrap();
    assert_eq!((held.run.clone(), held.pid, held.terminal.clone()), (None, Some(4242), Some(RunKey::terminal(3))), "stopping it never stops the shell");
    assert!(held.holder.starts_with("python3 in "), "{}", held.holder);
    assert_eq!(web.addr, Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
    assert_eq!(web.process.as_ref().map(|p| p.pid), Some(4242));
    assert!(!web.foreign);
    // A port a command declares, nobody on it: free, wanted by that command.
    let free = rows.iter().find(|r| r.port == 6080).expect("remotedesktopapp declares :6080");
    assert!(free.held.is_none());
    assert_eq!(free.wants.iter().map(|w| (w.key.cmd.as_str(), w.declared)).collect::<Vec<_>>(), [("compose:up", true)]);
    // The catalog's holder (the demo's port-forward) isn't foreign.
    assert!(rows.iter().find(|r| r.port == 5900).is_some_and(|r| r.held.is_some() && !r.foreign));
    assert!(rows.iter().all(|r| r.port != 5432), "the machine's other listeners only when asked");
    assert!(rows.windows(2).all(|w| w[0].port < w[1].port), "by number");

    let all = cx.read(|cx| s.read(cx).port_rows(true, cx));
    let pg = all.iter().find(|r| r.port == 5432).expect("with the machine");
    assert!(pg.foreign && pg.held.as_ref().is_some_and(|h| h.pid == Some(77)));
}

#[gpui::test]
fn the_machine_toggle_is_the_lists_and_kept(cx: &mut TestAppContext) {
    let shell = start(cx, "ports-toggle");
    let s = services(&shell, cx);
    scanned(&s, cx);
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    cx.update(|cx| sidebar.update(cx, |sb, cx| sb.show(Activity::Ports, cx)));
    cx.run_until_parked();
    let keys = |cx: &TestAppContext| cx.read(|cx| sidebar.read(cx).items(cx).into_iter().map(|i| i.key).collect::<Vec<_>>());
    assert!(keys(cx).contains(&"8000".to_string()) && !keys(cx).contains(&"5432".to_string()));
    cx.update(|cx| sidebar.update(cx, |sb, cx| sb.toggle_ports_machine(cx)));
    cx.run_until_parked();
    assert!(keys(cx).contains(&"5432".to_string()), "every listener");
    assert!(cx.read(|cx| sidebar.read(cx).state.ports_machine), "kept in its state");
    let free = cx.read(|cx| sidebar.read(cx).items(cx).into_iter().find(|i| i.key == "6080")).unwrap();
    assert!(!free.running && free.hint.as_deref().is_some_and(|h| h.ends_with("free")), "{:?}", free.hint);
}

fn main_window(shell: &Entity<Shell>, cx: &TestAppContext) -> AnyWindowHandle {
    cx.read(|cx| shell.read(cx).main_window().expect("main").into())
}

fn details(shell: &Entity<Shell>, cx: &TestAppContext) -> Option<Vec<ViewRef>> {
    cx.read(|cx| shell.read(cx).active_deck().read(cx).outside().map(<[ViewRef]>::to_vec))
}

/// Shows Ports with `port` selected; what its page draws.
fn select(shell: &Entity<Shell>, cx: &mut TestAppContext, port: &str) -> Vec<&'static str> {
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    cx.update(|cx| sidebar.update(cx, |s, cx| s.show_item(Activity::Ports, port, cx)));
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(main_window(shell, cx), cx);
    ["port-listening", "port-output", "port-wants", "port-forward", "port-foreign", "port-restart", "stop-holder"].into_iter().filter(|p| vcx.debug_bounds(p).is_some()).collect()
}

#[gpui::test]
fn ports_is_a_full_view_and_a_page_shows_what_its_port_is(cx: &mut TestAppContext) {
    let shell = start(cx, "ports-page");
    let s = services(&shell, cx);
    scanned(&s, cx);
    // A run's port: who, its output, who wants it, restart and stop.
    assert_eq!(select(&shell, cx, "5720"), ["port-listening", "port-output", "port-wants", "port-restart", "stop-holder"]);
    assert_eq!(details(&shell, cx), Some(vec![ViewAddr::Port { port: "5720".into() }.to_ref()]), "its page, no tab");
    // The catalog's port-forward: its target, nothing of devdeck's to show.
    assert_eq!(select(&shell, cx, "5900"), ["port-listening", "port-forward", "stop-holder"]);
    // Free: nobody on it, the command that declares it.
    assert_eq!(select(&shell, cx, "6080"), ["port-listening", "port-wants"]);
    // A program in a terminal: its output is the terminal's.
    assert_eq!(select(&shell, cx, "8000"), ["port-listening", "port-output", "stop-holder"]);
    // The machine's: not devdeck's.
    let sidebar = cx.read(|cx| shell.read(cx).active_sidebar());
    cx.update(|cx| sidebar.update(cx, |sb, cx| sb.toggle_ports_machine(cx)));
    assert_eq!(select(&shell, cx, "5432"), ["port-listening", "port-foreign", "stop-holder"]);
}

#[gpui::test]
fn copy_url_and_restart_act_on_the_port(cx: &mut TestAppContext) {
    let shell = start(cx, "ports-acts");
    let s = services(&shell, cx);
    select(&shell, cx, "5720");
    let runner = cx.read(|cx| s.read(cx).runner.clone());
    let key = RunKey::new("entityservice", "go:run");
    let runs = |cx: &TestAppContext| cx.read(|cx| runner.read(cx).run(&key).map(|r| r.runs));
    let before = runs(cx);
    let mut vcx = VisualTestContext::from_window(main_window(&shell, cx), cx);
    let copy = vcx.debug_bounds("port-copy").expect("Copy URL");
    vcx.simulate_click(copy.center(), Modifiers::none());
    assert_eq!(vcx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text())).as_deref(), Some("http://localhost:5720"));
    let restart = vcx.debug_bounds("port-restart").expect("Restart");
    vcx.simulate_click(restart.center(), Modifiers::none());
    vcx.executor().advance_clock(std::time::Duration::from_secs(5));
    vcx.run_until_parked();
    assert_eq!(runs(cx), before.map(|n| n + 1), "the same run, once more");
}

#[test]
fn the_http_check_reads_a_status_line_or_says_why_not() {
    use std::io::{Read, Write};
    assert_eq!(status_line("HTTP/1.1 200 OK\r\nServer: x\r\n").as_deref(), Some("200 OK"));
    assert_eq!(status_line("HTTP/1.0 404 Not Found\r\n").as_deref(), Some("404 Not Found"));
    assert_eq!(status_line("SSH-2.0-OpenSSH_9.6\r\n"), None);
    assert_eq!(status_line("HTTP/1.1 abc\r\n"), None);
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut c, _) = l.accept().unwrap();
        let mut buf = [0u8; 256];
        let n = c.read(&mut buf).unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HEAD / HTTP/1.0"), "a HEAD");
        c.write_all(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
    });
    let got = http_probe(port).expect("an answer");
    server.join().unwrap();
    assert!(got.starts_with("204 No Content · ") && got.ends_with(" ms"), "{got}");
    let gone = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    assert_eq!(http_probe(gone), Err("no answer".into()));
}
