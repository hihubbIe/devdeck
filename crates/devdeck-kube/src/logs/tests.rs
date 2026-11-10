//! Logs streams against a mock API: bodies given whole or kept open and fed by the test,
//! refusals as the API words them, on paused time.

use super::*;
use bytes::Bytes;
use http_body_util::channel::{Channel, Sender as BodySender};
use http_body_util::{Either, Full};
use serde_json::json;
use std::convert::Infallible;
use std::io;
use std::sync::{Arc, Mutex};

/// The body of an open logs stream: what the test sends is what the stream reads.
type Body = BodySender<Bytes, io::Error>;

#[derive(Clone)]
enum Reply {
    /// A body kept open, handed to the test.
    Open,
    /// A whole body, then its end.
    Whole(Vec<u8>),
    /// A `Status` with this code and message.
    Status(u16, &'static str),
}

struct Mock {
    /// Answered in order; the last one again and again.
    replies: Mutex<Vec<Reply>>,
    /// The query of each request.
    queries: Mutex<Vec<String>>,
    opened: async_channel::Sender<Body>,
}

impl Mock {
    fn answer(&self, req: &http::Request<kube::client::Body>) -> http::Response<Either<Full<Bytes>, Channel<Bytes, io::Error>>> {
        assert_eq!(req.uri().path(), "/api/v1/namespaces/dev35/pods/api-5d8f7c9b6-x2k4p/log");
        self.queries.lock().unwrap().push(req.uri().query().unwrap_or_default().into());
        let reply = {
            let mut replies = self.replies.lock().unwrap();
            if replies.len() > 1 { replies.remove(0) } else { replies[0].clone() }
        };
        let (code, body) = match reply {
            Reply::Open => {
                let (body, channel) = Channel::new(16);
                let _ = self.opened.try_send(body);
                return http::Response::new(Either::Right(channel));
            }
            Reply::Whole(bytes) => (200, bytes),
            Reply::Status(code, message) => {
                (code, json!({"apiVersion": "v1", "kind": "Status", "metadata": {}, "status": "Failure", "message": message, "reason": "BadRequest", "code": code}).to_string().into_bytes())
            }
        };
        let mut response = http::Response::new(Either::Left(Full::new(Bytes::from(body))));
        *response.status_mut() = http::StatusCode::from_u16(code).unwrap();
        response
    }
}

struct Logs {
    rx: async_channel::Receiver<LogUpdate>,
    handle: WatchHandle,
    mock: Arc<Mock>,
    opened: async_channel::Receiver<Body>,
}

impl Logs {
    /// The logs of container `api` of a pod of dev35 on km011gcp, as [`logs`] streams them,
    /// whose requests are answered with `replies`.
    fn start(replies: Vec<Reply>, from: LogFrom, previous: bool) -> Logs {
        let (opened_tx, opened) = async_channel::unbounded();
        let mock = Arc::new(Mock { replies: Mutex::new(replies), queries: Mutex::default(), opened: opened_tx });
        let m = mock.clone();
        let service = tower::service_fn(move |req: http::Request<kube::client::Body>| {
            let response = m.answer(&req);
            async move { Ok::<_, Infallible>(response) }
        });
        let client = Client::new(service, "dev35");
        let (tx, rx) = async_channel::unbounded();
        let out = Out::new(tx.clone());
        let task = tokio::spawn(until_closed(tx, async move {
            let target = Target::for_tests("dev35", "km011gcp");
            stream(client, &target, "api-5d8f7c9b6-x2k4p", "api", from, previous, out).await;
        }));
        Logs { rx, handle: WatchHandle { task: task.abort_handle() }, mock, opened }
    }

    async fn next(&self) -> LogUpdate {
        tokio::time::timeout(Duration::from_secs(60), self.rx.recv()).await.expect("an update").expect("the stream runs")
    }

    /// The channel closes: the stream is over.
    async fn closed(&self) {
        let next = tokio::time::timeout(Duration::from_secs(60), self.rx.recv()).await.expect("the channel closes");
        assert_eq!(next.ok(), None);
    }

    async fn body(&self) -> Body {
        tokio::time::timeout(Duration::from_secs(600), self.opened.recv()).await.expect("a request").unwrap()
    }

    fn queries(&self) -> Vec<String> {
        self.mock.queries.lock().unwrap().clone()
    }
}

/// 2026-11-10T09:00:00Z.
const NINE: i64 = 1_794_301_200;

/// Epoch nanoseconds `s` seconds and `ns` nanoseconds after [`NINE`].
fn at(s: i64, ns: i64) -> i64 {
    (NINE + s) * NANOS + ns
}

/// A line as the server sends it, at `s` seconds and `ms` milliseconds after [`NINE`].
fn raw(s: u32, ms: u32, text: &str) -> String {
    format!("2026-11-10T09:00:{s:02}.{ms:03}000000Z {text}\n")
}

fn line(s: i64, ms: i64, text: &str) -> LogLine {
    LogLine { at: Some(at(s, ms * 1_000_000)), text: text.into(), level: log_level(text) }
}

async fn send(body: &mut Body, data: impl Into<Bytes>) {
    body.send_data(data.into()).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_tail_parses_timestamps_and_levels() {
    let body = "2026-11-10T09:00:01.123456789Z ERROR boom\n2026-11-10T09:00:02Z {\"level\":\"warn\",\"msg\":\"slow\"}\nno timestamp here\n";
    let l = Logs::start(vec![Reply::Whole(body.into())], LogFrom::Tail(200), false);
    let expected = vec![
        LogLine { at: Some(at(1, 123_456_789)), text: "ERROR boom".into(), level: Level::Error },
        LogLine { at: Some(at(2, 0)), text: "{\"level\":\"warn\",\"msg\":\"slow\"}".into(), level: Level::Warn },
        LogLine { at: None, text: "no timestamp here".into(), level: Level::Plain },
    ];
    assert_eq!(l.next().await, LogUpdate::Lines(expected), "one batch, nanoseconds kept");
    assert_eq!(l.next().await, LogUpdate::Ended);
    l.closed().await;
    let query = &l.queries()[0];
    for part in ["container=api", "follow=true", "tailLines=200", "timestamps=true"] {
        assert!(query.contains(part), "{query}");
    }
    assert!(!query.contains("previous") && !query.contains("sinceTime"), "{query}");
}

#[tokio::test(start_paused = true)]
async fn a_followed_stream_delivers_later_lines() {
    let l = Logs::start(vec![Reply::Open], LogFrom::Tail(200), false);
    let mut body = l.body().await;
    send(&mut body, raw(1, 0, "first")).await;
    assert_eq!(l.next().await, LogUpdate::Lines(vec![line(1, 0, "first")]));
    tokio::time::sleep(Duration::from_secs(10)).await;
    send(&mut body, raw(11, 0, "later")).await;
    assert_eq!(l.next().await, LogUpdate::Lines(vec![line(11, 0, "later")]));
    drop(body);
    assert_eq!(l.next().await, LogUpdate::Ended, "the container stopped");
    l.closed().await;
}

#[tokio::test(start_paused = true)]
async fn lines_come_in_batches() {
    let l = Logs::start(vec![Reply::Open], LogFrom::Tail(200), false);
    let mut body = l.body().await;
    for i in 0..3 {
        send(&mut body, raw(1, i, &format!("line {i}"))).await;
    }
    assert_eq!(l.next().await, LogUpdate::Lines((0..3).map(|i| line(1, i, &format!("line {i}"))).collect()));
    // A line every 20 ms: a batch at most every 50 ms, not one per line.
    for i in 0..10 {
        send(&mut body, raw(2, i * 20, "busy")).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut batches = 0;
    let mut lines = 0;
    while let Ok(LogUpdate::Lines(batch)) = l.rx.try_recv() {
        batches += 1;
        lines += batch.len();
    }
    assert_eq!(lines, 10);
    assert!((3..=5).contains(&batches), "{batches} batches");
}

#[tokio::test(start_paused = true)]
async fn forbidden_is_refused_once() {
    let forbidden = "pods \"api-5d8f7c9b6-x2k4p\" is forbidden: User \"system:serviceaccount:x\" cannot get resource \"pods/log\" in API group \"\" in the namespace \"dev35\"";
    let l = Logs::start(vec![Reply::Status(403, forbidden)], LogFrom::Tail(200), false);
    assert_eq!(l.next().await, LogUpdate::Refused(LogRefusal::Forbidden("your account can't read logs in dev35".into())));
    l.closed().await;
    tokio::time::sleep(Duration::from_secs(100)).await;
    assert_eq!(l.queries().len(), 1, "not retried");
}

/// What a request answered with a 400 of `message` sends.
async fn refusal(message: &'static str, previous: bool) -> LogUpdate {
    let l = Logs::start(vec![Reply::Status(400, message)], LogFrom::Tail(200), previous);
    let update = l.next().await;
    l.closed().await;
    assert_eq!(l.queries().len(), 1, "not retried");
    update
}

#[tokio::test(start_paused = true)]
async fn bad_requests_say_why() {
    assert_eq!(
        refusal("container \"api\" in pod \"api-5d8f7c9b6-x2k4p\" is waiting to start: ContainerCreating", false).await,
        LogUpdate::Refused(LogRefusal::Waiting("waiting to start: ContainerCreating".into()))
    );
    assert_eq!(
        refusal("container \"api\" in pod \"api-5d8f7c9b6-x2k4p\" is waiting to start: trying and failing to pull image", false).await,
        LogUpdate::Refused(LogRefusal::Waiting("waiting to start: trying and failing to pull image".into()))
    );
    assert_eq!(
        refusal("previous terminated container \"api\" in pod \"api-5d8f7c9b6-x2k4p\" not found", true).await,
        LogUpdate::Refused(LogRefusal::NoPrevious("no previous run of api to read".into()))
    );
    assert_eq!(refusal("container api is not valid for pod api-5d8f7c9b6-x2k4p", false).await, LogUpdate::Refused(LogRefusal::NoContainer("no container api in api-5d8f7c9b6-x2k4p".into())));
    assert_eq!(
        refusal("a container name must be specified for pod api-5d8f7c9b6-x2k4p, choose one of: [api sidecar]", false).await,
        LogUpdate::Refused(LogRefusal::Other("km011gcp (dev35): a container name must be specified for pod api-5d8f7c9b6-x2k4p, choose one of: [api sidecar] (400)".into()))
    );
}

#[tokio::test(start_paused = true)]
async fn a_gone_pod_is_gone() {
    let l = Logs::start(vec![Reply::Status(404, "pods \"api-5d8f7c9b6-x2k4p\" not found")], LogFrom::Tail(200), false);
    assert_eq!(l.next().await, LogUpdate::Gone);
    l.closed().await;
}

#[tokio::test(start_paused = true)]
async fn a_server_error_is_a_problem_tried_again() {
    let l = Logs::start(vec![Reply::Status(503, "the server is currently unable to handle the request"), Reply::Whole(raw(1, 0, "back").into())], LogFrom::Tail(200), false);
    assert_eq!(l.next().await, LogUpdate::Problem(Some("km011gcp (dev35): the server is currently unable to handle the request (503)".into())));
    assert_eq!(l.next().await, LogUpdate::Problem(None));
    assert_eq!(l.next().await, LogUpdate::Lines(vec![line(1, 0, "back")]));
    assert_eq!(l.next().await, LogUpdate::Ended);
}

#[tokio::test(start_paused = true)]
async fn a_problem_clears_when_the_stream_connects_again_without_lines() {
    let l = Logs::start(vec![Reply::Status(503, "the server is currently unable to handle the request"), Reply::Open], LogFrom::Tail(200), false);
    assert!(matches!(l.next().await, LogUpdate::Problem(Some(_))));
    let _body = l.body().await;
    assert_eq!(l.next().await, LogUpdate::Problem(None), "a quiet pod's warning doesn't wait for its next line");
}

#[tokio::test(start_paused = true)]
async fn a_previous_run_is_read_once_not_followed() {
    let l = Logs::start(vec![Reply::Whole(raw(1, 0, "panic: boom").into())], LogFrom::Tail(200), true);
    assert_eq!(l.next().await, LogUpdate::Lines(vec![line(1, 0, "panic: boom")]));
    assert_eq!(l.next().await, LogUpdate::Ended);
    l.closed().await;
    let query = &l.queries()[0];
    assert!(query.contains("previous=true"), "{query}");
    assert!(!query.contains("follow"), "{query}");
}

#[tokio::test(start_paused = true)]
async fn since_asks_from_the_second_and_skips_what_was_seen() {
    let replay = [raw(1, 100, "older"), raw(1, 700, "same"), raw(1, 700, "same"), raw(1, 800, "after")].concat();
    let from = LogFrom::Since { at: at(1, 700_000_000), seen: 1, cap: 500 };
    let l = Logs::start(vec![Reply::Whole(replay.into())], from, false);
    assert_eq!(l.next().await, LogUpdate::Lines(vec![line(1, 700, "same"), line(1, 800, "after")]), "the unseen one of two identical lines kept");
    let query = &l.queries()[0];
    assert!(query.contains("sinceTime=2026-11-10T09%3A00%3A01Z"), "truncated, not rounded up: {query}");
    assert!(query.contains("tailLines=500"), "{query}");
}

#[tokio::test(start_paused = true)]
async fn a_dropped_stream_resumes_from_its_last_line() {
    let l = Logs::start(vec![Reply::Open], LogFrom::Tail(200), false);
    let mut body = l.body().await;
    send(&mut body, [raw(1, 100, "a"), raw(1, 700, "same")].concat()).await;
    assert_eq!(l.next().await, LogUpdate::Lines(vec![line(1, 100, "a"), line(1, 700, "same")]));
    body.abort(io::Error::new(io::ErrorKind::ConnectionReset, "connection reset by peer"));
    assert_eq!(l.next().await, LogUpdate::Problem(Some("can't reach km011gcp (dev35): connection reset by peer".into())));
    let mut body = l.body().await;
    // The server sends again what came since the second asked.
    send(&mut body, [raw(1, 100, "a"), raw(1, 700, "same"), raw(1, 700, "same"), raw(1, 800, "b")].concat()).await;
    assert_eq!(l.next().await, LogUpdate::Problem(None));
    assert_eq!(l.next().await, LogUpdate::Lines(vec![line(1, 700, "same"), line(1, 800, "b")]), "nothing doubled, nothing lost");
    let query = &l.queries()[1];
    assert!(query.contains("sinceTime=2026-11-10T09%3A00%3A01Z"), "{query}");
    assert!(query.contains("tailLines=1000") && query.contains("follow=true"), "{query}");
}

#[tokio::test(start_paused = true)]
async fn a_silent_stream_is_resumed_after_five_minutes() {
    let l = Logs::start(vec![Reply::Open], LogFrom::Tail(200), false);
    let mut first = l.body().await;
    send(&mut first, raw(1, 0, "hello")).await;
    assert_eq!(l.next().await, LogUpdate::Lines(vec![line(1, 0, "hello")]));
    let start = Instant::now();
    let mut second = l.body().await;
    assert!(start.elapsed() >= Duration::from_secs(299), "{:?}", start.elapsed());
    assert!(l.queries()[1].contains("sinceTime=2026-11-10T09%3A00%3A01Z"), "{:?}", l.queries());
    send(&mut second, [raw(1, 0, "hello"), raw(9, 0, "again")].concat()).await;
    assert_eq!(l.next().await, LogUpdate::Lines(vec![line(9, 0, "again")]), "no problem shown, nothing doubled");
}

#[tokio::test(start_paused = true)]
async fn invalid_utf8_and_endless_lines_are_kept_in_bounds() {
    let l = Logs::start(vec![Reply::Open], LogFrom::Tail(200), false);
    let mut body = l.body().await;
    let mut bad = b"2026-11-10T09:00:01Z bad \xff\xfe bytes\n".to_vec();
    bad.extend_from_slice(b"2026-11-10T09:00:02Z ");
    send(&mut body, bad).await;
    // 10 KB without a newline, in chunks, then the next line.
    for _ in 0..10 {
        send(&mut body, vec![b'x'; 1024]).await;
    }
    send(&mut body, format!("\n{}", raw(3, 0, "after"))).await;
    let LogUpdate::Lines(lines) = l.next().await else { panic!("lines") };
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0].text, "bad \u{fffd}\u{fffd} bytes");
    assert_eq!(lines[1].at, Some(at(2, 0)));
    assert!(lines[1].text.ends_with(CUT), "marked as cut");
    assert_eq!(lines[1].text.len(), MAX_LINE - "2026-11-10T09:00:02Z ".len() + CUT.len());
    assert_eq!(lines[2], line(3, 0, "after"), "the stream goes on");
}

#[tokio::test(start_paused = true)]
async fn dropping_the_handle_ends_the_request() {
    let l = Logs::start(vec![Reply::Open], LogFrom::Tail(200), false);
    let mut body = l.body().await;
    send(&mut body, raw(1, 0, "hello")).await;
    l.next().await;
    drop(l.handle);
    let closed = tokio::time::timeout(Duration::from_secs(1), l.rx.recv()).await.expect("the channel closes");
    assert!(closed.is_err());
    tokio::task::yield_now().await;
    assert!(body.send_data(Bytes::from(raw(2, 0, "nobody"))).await.is_err(), "the body is dropped");
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(l.mock.queries.lock().unwrap().len(), 1);
}
