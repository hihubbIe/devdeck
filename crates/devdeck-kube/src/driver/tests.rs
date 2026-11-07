//! The driver against a mock API: a list answered from a script, watches whose bodies
//! stay open and carry what a test sends, on paused time.

use super::*;
use bytes::Bytes;
use http_body_util::channel::{Channel, Sender as BodySender};
use http_body_util::{Either, Full};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The body of an open watch: each line sent is an event.
type Events = BodySender<Bytes, io::Error>;

#[derive(Clone)]
enum Reply {
    List(Vec<Value>),
    Status(u16),
}

struct Mock {
    /// Answered in order; the last one again and again.
    lists: Mutex<Vec<Reply>>,
    requests: AtomicUsize,
    watches: async_channel::Sender<Events>,
}

impl Mock {
    fn answer(&self, req: &http::Request<kube::client::Body>) -> http::Response<Either<Full<Bytes>, Channel<Bytes, io::Error>>> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        if req.uri().query().unwrap_or_default().contains("watch=true") {
            let (events, body) = Channel::new(16);
            let _ = self.watches.try_send(events);
            return http::Response::new(Either::Right(body));
        }
        let reply = {
            let mut lists = self.lists.lock().unwrap();
            if lists.len() > 1 { lists.remove(0) } else { lists[0].clone() }
        };
        let (code, body) = match reply {
            Reply::List(items) => (200, json!({"apiVersion": "apps/v1", "kind": "DeploymentList", "metadata": {"resourceVersion": "100"}, "items": items})),
            Reply::Status(code) => (
                code,
                json!({"apiVersion": "v1", "kind": "Status", "metadata": {}, "status": "Failure", "message": "deployments.apps is forbidden", "reason": "Forbidden", "code": code}),
            ),
        };
        let mut response = http::Response::new(Either::Left(Full::new(Bytes::from(body.to_string()))));
        *response.status_mut() = http::StatusCode::from_u16(code).unwrap();
        response
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

struct Watch {
    rx: async_channel::Receiver<EnvUpdate>,
    handle: WatchHandle,
    task: tokio::task::JoinHandle<()>,
    mock: Arc<Mock>,
    /// The body of each watch the driver opens, in order.
    opened: async_channel::Receiver<Events>,
}

impl Watch {
    /// A watch of dev35 on km011gcp, whose lists are answered with `lists`.
    fn start(lists: Vec<Reply>) -> Watch {
        let (watches, opened) = async_channel::unbounded();
        let mock = Arc::new(Mock { lists: Mutex::new(lists), requests: AtomicUsize::new(0), watches });
        let m = mock.clone();
        let service = tower::service_fn(move |req: http::Request<kube::client::Body>| {
            let response = m.answer(&req);
            async move { Ok::<_, Infallible>(response) }
        });
        let client = Client::new(service, "dev35");
        let (tx, rx) = async_channel::unbounded();
        let mut out = Out::new(tx.clone());
        let target = Target::for_tests("dev35", "km011gcp");
        let task = tokio::spawn(until_closed(tx, async move { feed(client, &target, &mut out).await }));
        Watch { rx, handle: WatchHandle { task: task.abort_handle() }, task, mock, opened }
    }

    async fn next(&self) -> EnvUpdate {
        tokio::time::timeout(Duration::from_secs(60), self.rx.recv()).await.expect("an update").expect("the watch runs")
    }

    /// Nothing is sent within a second.
    async fn quiet(&self) {
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(self.rx.try_recv().ok(), None);
    }

    async fn events(&self) -> Events {
        tokio::time::timeout(Duration::from_secs(60), self.opened.recv()).await.expect("a watch").unwrap()
    }
}

/// A deployment as the API returns it, one container `registry/<name>:<tag>`.
fn dep(name: &str, tag: &str, version: u32) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {"name": name, "namespace": "dev35", "generation": 1, "resourceVersion": version.to_string(), "creationTimestamp": "2026-09-01T08:00:00Z"},
        "spec": {
            "replicas": 1,
            "selector": {"matchLabels": {"app": name}},
            "template": {"metadata": {"labels": {"app": name}}, "spec": {"containers": [{"name": name, "image": format!("registry/{name}:{tag}")}]}}
        },
        "status": {"observedGeneration": 1, "replicas": 1, "readyReplicas": 1, "updatedReplicas": 1, "availableReplicas": 1}
    })
}

fn model(v: &Value) -> Deployment {
    convert::deployment(&serde_json::from_value(v.clone()).unwrap()).unwrap()
}

fn deployments(vs: &[&Value]) -> EnvUpdate {
    EnvUpdate::Deployments(vs.iter().map(|v| model(v)).collect())
}

async fn send(events: &mut Events, kind: &str, object: &Value) {
    let line = json!({"type": kind, "object": object}).to_string() + "\n";
    events.send_data(Bytes::from(line)).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_list_then_changes_give_snapshots() {
    let (a, b) = (dep("api", "1.0", 10), dep("billing", "2.0", 11));
    let w = Watch::start(vec![Reply::List(vec![b.clone(), a.clone()])]);
    assert_eq!(w.next().await, EnvUpdate::Problem(None));
    assert_eq!(w.next().await, deployments(&[&a, &b]), "alphabetical");
    let mut events = w.events().await;
    let c = dep("crawler", "0.9", 101);
    send(&mut events, "ADDED", &c).await;
    assert_eq!(w.next().await, deployments(&[&a, &b, &c]));
    let b2 = dep("billing", "2.1", 102);
    send(&mut events, "MODIFIED", &b2).await;
    assert_eq!(w.next().await, deployments(&[&a, &b2, &c]));
    send(&mut events, "DELETED", &dep("api", "1.0", 103)).await;
    assert_eq!(w.next().await, deployments(&[&b2, &c]));
    w.quiet().await;
}

#[tokio::test(start_paused = true)]
async fn changes_that_leave_the_model_as_it_was_send_nothing() {
    let a = dep("api", "1.0", 10);
    let w = Watch::start(vec![Reply::List(vec![a.clone()])]);
    assert_eq!(w.next().await, EnvUpdate::Problem(None));
    assert_eq!(w.next().await, deployments(&[&a]));
    let mut events = w.events().await;
    let mut status_only = dep("api", "1.0", 101);
    status_only["status"]["availableReplicas"] = json!(0);
    status_only["metadata"]["annotations"] = json!({"touched": "yes"});
    send(&mut events, "MODIFIED", &status_only).await;
    w.quiet().await;
    let a2 = dep("api", "1.1", 102);
    send(&mut events, "MODIFIED", &a2).await;
    assert_eq!(w.next().await, deployments(&[&a2]), "the watch still runs");
}

#[tokio::test(start_paused = true)]
async fn changes_in_quick_succession_are_one_update() {
    let a = dep("api", "1.0", 10);
    let w = Watch::start(vec![Reply::List(vec![a.clone()])]);
    w.next().await;
    w.next().await;
    let mut events = w.events().await;
    for (i, tag) in ["1.1", "1.2", "1.3"].into_iter().enumerate() {
        send(&mut events, "MODIFIED", &dep("api", tag, 101 + i as u32)).await;
    }
    assert_eq!(w.next().await, deployments(&[&dep("api", "1.3", 0)]));
    w.quiet().await;
}

#[tokio::test(start_paused = true)]
async fn steady_churn_still_sends_at_the_settle_pace() {
    let a = dep("api", "1.0", 10);
    let w = Watch::start(vec![Reply::List(vec![a.clone()])]);
    w.next().await;
    w.next().await;
    let mut events = w.events().await;
    // A change every 40 ms for a second: never a 100 ms pause.
    for i in 0..25u32 {
        send(&mut events, "MODIFIED", &dep("api", &format!("1.{i}"), 101 + i)).await;
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    let mut sent = 0;
    while let Ok(EnvUpdate::Deployments(_)) = w.rx.try_recv() {
        sent += 1;
    }
    assert!((8..=11).contains(&sent), "one update about every 100 ms, not none ({sent})");
}

#[tokio::test(start_paused = true)]
async fn forbidden_shows_once_and_retries_with_a_capped_backoff() {
    let w = Watch::start(vec![Reply::Status(403)]);
    assert_eq!(w.next().await, EnvUpdate::Problem(Some("your account can't list deployments in dev35".into())));
    tokio::time::sleep(Duration::from_secs(100)).await;
    // Lists at 0, 1, 3, 7, 15, 31, 61 and 91 s.
    assert_eq!(w.mock.requests(), 8);
    assert_eq!(w.rx.try_recv().ok(), None, "the same problem isn't sent again");
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert_eq!(w.mock.requests(), 18, "every 30 s from then on");
}

#[tokio::test(start_paused = true)]
async fn after_an_error_a_new_list_clears_the_problem_and_what_was_deleted_meanwhile() {
    let (a, b) = (dep("api", "1.0", 10), dep("billing", "2.0", 11));
    let w = Watch::start(vec![Reply::List(vec![a.clone(), b.clone()]), Reply::List(vec![b.clone()])]);
    assert_eq!(w.next().await, EnvUpdate::Problem(None));
    assert_eq!(w.next().await, deployments(&[&a, &b]));
    let events = w.events().await;
    events.abort(io::Error::new(io::ErrorKind::ConnectionReset, "connection reset by peer"));
    assert_eq!(w.next().await, EnvUpdate::Problem(Some("can't reach km011gcp (dev35): connection reset by peer".into())));
    assert_eq!(w.next().await, EnvUpdate::Problem(None));
    assert_eq!(w.next().await, deployments(&[&b]));
    assert_eq!(w.mock.requests(), 4, "list, watch, list, watch");
}

#[tokio::test(start_paused = true)]
async fn dropping_the_handle_ends_the_watch() {
    let w = Watch::start(vec![Reply::List(vec![dep("api", "1.0", 10)])]);
    w.next().await;
    w.next().await;
    let _events = w.events().await;
    let requests = w.mock.requests();
    drop(w.handle);
    let closed = tokio::time::timeout(Duration::from_secs(1), w.rx.recv()).await.expect("the channel closes");
    assert!(closed.is_err());
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(w.mock.requests(), requests);
}

#[tokio::test(start_paused = true)]
async fn dropping_the_receiver_ends_the_watch() {
    let w = Watch::start(vec![Reply::List(vec![dep("api", "1.0", 10)])]);
    w.next().await;
    drop(w.rx);
    tokio::time::timeout(Duration::from_secs(1), w.task).await.expect("the task ends").unwrap();
}

/// The real path, from a kubeconfig to a problem, against a local port nothing listens on
/// (no cluster is contacted): the runtime starts, one client serves both envs of the file,
/// and a refused connection reads as one.
#[test]
fn a_cluster_that_refuses_connections_is_unreachable() {
    let dir = devdeck_core::testing::TempDir::new("kube-refused");
    let config = "apiVersion: v1\nkind: Config\nclusters:\n- name: km011gcp\n  cluster:\n    server: http://127.0.0.1:1\nusers:\n- name: tailscale-auth\n  user:\n    token: fake-token\ncontexts:\n- name: dev35\n  context:\n    cluster: km011gcp\n    user: tailscale-auth\n    namespace: dev35\n- name: dev36\n  context:\n    cluster: km011gcp\n    user: tailscale-auth\n    namespace: dev36\n";
    std::fs::write(dir.join("dev-km011gcp"), config).unwrap();
    let (configs, problems) = crate::Kubeconfigs::load(&dir);
    assert!(problems.is_empty(), "{problems:?}");
    let mut watches = Vec::new();
    for env in ["dev35", "dev36"] {
        let (rx, handle) = watch(&configs.target(env, env).unwrap());
        let EnvUpdate::Problem(Some(problem)) = rx.recv_blocking().unwrap() else { panic!("a problem") };
        assert!(problem.starts_with(&format!("can't reach km011gcp ({env}): ")), "{problem}");
        assert!(problem.to_lowercase().contains("refused"), "{problem}");
        watches.push((rx, handle));
    }
    let file = dir.join("dev-km011gcp");
    let clients = || CLIENTS.lock().unwrap().iter().filter(|((f, _, _), _)| *f == file).map(|(_, s)| s.users).collect::<Vec<_>>();
    assert_eq!(clients(), [2], "one client, both watches using it");
    drop(watches);
    let end = std::time::Instant::now() + Duration::from_secs(2);
    while !clients().is_empty() {
        assert!(std::time::Instant::now() < end, "the client goes with its last watch");
        std::thread::sleep(Duration::from_millis(10));
    }
}
