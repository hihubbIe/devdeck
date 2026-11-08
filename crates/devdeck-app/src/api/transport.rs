//! The socket's side of the API: one thread accepts (blocked while nobody connects, so
//! idle costs nothing, R1.3), one thread per connection reads a message, hands it to a
//! [`Sink`] and writes the answer. One request at a time per connection. The sink is
//! devdeck's UI thread in the app, a stub in tests, so this runs without GPUI.

use super::identity::{Peer, peer};
use devdeck_api::client::read_message;
use devdeck_api::rpc::{self, Request, Response};
use devdeck_api::{ApiError, ErrorKind};
use serde_json::Value;
use std::collections::BTreeSet;
use std::io::{BufReader, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

/// Connections at most, of which outside callers.
pub const MAX_CONNECTIONS: usize = 32;
pub const MAX_OUTSIDE: usize = 8;

/// A connection, as the sink took it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opened {
    pub id: u64,
    pub outside: bool,
}

/// Where requests go. Calls block the connection's thread until answered.
pub trait Sink: Send + Sync + 'static {
    /// A new connection; None refuses it.
    fn open(&self, peer: Peer) -> Option<Opened>;
    fn call(&self, conn: u64, method: String, params: Value) -> Result<Value, ApiError>;
    /// A message without an id: never answered.
    fn notification(&self, conn: u64, method: String);
    fn close(&self, conn: u64);
    /// A connection refused before it was opened (another user, too many).
    fn refused(&self, why: String);
}

/// Stops accepting when dropped or stopped (connections left end with their clients).
pub struct Stop(Option<OwnedFd>);

impl Stop {
    pub fn stop(&mut self) {
        if let Some(fd) = self.0.take() {
            let _ = rustix::net::shutdown(&fd, rustix::net::Shutdown::Both);
        }
    }
}

impl Drop for Stop {
    fn drop(&mut self) {
        self.stop();
    }
}

static METHODS: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| devdeck_api::catalog().into_iter().map(|m| m.name).collect());

/// Serves `listener` on its own thread.
pub fn serve(listener: UnixListener, sink: Arc<dyn Sink>) -> std::io::Result<Stop> {
    let stop = listener.as_fd().try_clone_to_owned()?;
    let counts = Arc::new((AtomicUsize::new(0), AtomicUsize::new(0)));
    std::thread::Builder::new().name("devdeck-api".into()).spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let (sink, counts) = (sink.clone(), counts.clone());
            let _ = std::thread::Builder::new().name("devdeck-api-conn".into()).spawn(move || connection(stream, &*sink, &counts));
        }
    })?;
    Ok(Stop(Some(stop)))
}

fn connection(stream: UnixStream, sink: &dyn Sink, counts: &(AtomicUsize, AtomicUsize)) {
    let (all, outside) = counts;
    if all.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
        all.fetch_sub(1, Ordering::SeqCst);
        sink.refused("too many connections".into());
        return;
    }
    let opened = match peer(&stream) {
        Ok(p) => sink.open(p),
        Err(why) => {
            sink.refused(why);
            None
        }
    };
    let counted_outside = opened.is_some_and(|o| o.outside);
    let within = !counted_outside || outside.fetch_add(1, Ordering::SeqCst) < MAX_OUTSIDE;
    if let Some(o) = opened {
        if within {
            serve_connection(stream, sink, o.id);
        } else {
            sink.refused("too many outside connections".into());
        }
        sink.close(o.id);
    }
    if counted_outside {
        outside.fetch_sub(1, Ordering::SeqCst);
    }
    all.fetch_sub(1, Ordering::SeqCst);
}

fn serve_connection(stream: UnixStream, sink: &dyn Sink, conn: u64) {
    let Ok(read) = stream.try_clone() else { return };
    let mut reader = BufReader::new(read);
    let mut writer = stream;
    let mut buf = Vec::new();
    loop {
        let (answer, close) = match read_message(&mut reader, &mut buf) {
            Ok(false) | Err(_) if buf.is_empty() => return,
            Err(e) => (Some(Response::err(Value::Null, ApiError::invalid(e.to_string()).to_rpc_with(rpc::INVALID_REQUEST))), true),
            Ok(_) => answer(&buf, sink, conn),
        };
        if let Some(a) = answer {
            let Ok(mut line) = serde_json::to_vec(&a) else { return };
            line.push(b'\n');
            if writer.write_all(&line).is_err() {
                return;
            }
        }
        if close {
            return;
        }
    }
}

/// The answer to one message (None for a notification), and whether to close after it.
fn answer(msg: &[u8], sink: &dyn Sink, conn: u64) -> (Option<Response>, bool) {
    let fail = |id: Value, code: i64, kind: ErrorKind, m: &str| (Some(Response::err(id, ApiError::new(kind, m).to_rpc_with(code))), false);
    let value: Value = match serde_json::from_slice(msg) {
        Ok(v) => v,
        Err(e) => return fail(Value::Null, rpc::PARSE_ERROR, ErrorKind::Invalid, &e.to_string()),
    };
    if value.is_array() {
        return fail(Value::Null, rpc::INVALID_REQUEST, ErrorKind::Invalid, "batches aren't supported");
    }
    let req: Request = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => return fail(Value::Null, rpc::INVALID_REQUEST, ErrorKind::Invalid, &e.to_string()),
    };
    let Some(id) = req.id else {
        sink.notification(conn, req.method);
        return (None, false);
    };
    if req.jsonrpc != rpc::VERSION {
        return fail(id, rpc::INVALID_REQUEST, ErrorKind::Invalid, "jsonrpc must be \"2.0\"");
    }
    if !METHODS.contains(req.method.as_str()) {
        return fail(id, rpc::METHOD_NOT_FOUND, ErrorKind::NotFound, &format!("no method {}", req.method));
    }
    let params = match req.params {
        None | Some(Value::Null) => Value::Object(Default::default()),
        Some(p @ Value::Object(_)) => p,
        Some(_) => return fail(id, rpc::INVALID_PARAMS, ErrorKind::Invalid, "params must be an object"),
    };
    match sink.call(conn, req.method, params) {
        Ok(v) => (Some(Response::ok(id, v)), false),
        // The caller's shell is gone: this connection is no longer it.
        Err(e) => {
            let close = e.kind == ErrorKind::Expired;
            (Some(Response::err(id, e.to_rpc())), close)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use devdeck_api::Client;
    use devdeck_api::client::ClientError;
    use devdeck_core::testing::TempDir;
    use serde_json::json;
    use std::io::BufRead;
    use std::sync::Mutex;

    /// Opens everyone (as outside when told), echoes calls, records the rest.
    #[derive(Default)]
    struct Stub {
        outside: bool,
        log: Mutex<Vec<String>>,
    }

    impl Sink for Stub {
        fn open(&self, peer: Peer) -> Option<Opened> {
            self.log.lock().unwrap().push(format!("open {}", peer.pid.is_some()));
            Some(Opened { id: 1, outside: self.outside })
        }
        fn call(&self, _: u64, method: String, params: Value) -> Result<Value, ApiError> {
            match method.as_str() {
                "session.whoami" => Err(ApiError::new(ErrorKind::Expired, "the shell ended")),
                _ => Ok(json!({ "method": method, "params": params })),
            }
        }
        fn notification(&self, _: u64, method: String) {
            self.log.lock().unwrap().push(format!("notification {method}"));
        }
        fn close(&self, _: u64) {
            self.log.lock().unwrap().push("close".into());
        }
        fn refused(&self, why: String) {
            self.log.lock().unwrap().push(format!("refused {why}"));
        }
    }

    fn start(stub: Stub) -> (TempDir, std::path::PathBuf, Arc<Stub>, Stop) {
        let dir = TempDir::new("api-tr");
        let path = dir.join("s.sock");
        let stub = Arc::new(stub);
        let stop = serve(UnixListener::bind(&path).unwrap(), stub.clone()).unwrap();
        (dir, path, stub, stop)
    }

    /// Sends raw lines, reads one answer per line given `answers`.
    fn raw(path: &std::path::Path, lines: &[&str], answers: usize) -> Vec<Value> {
        let mut s = UnixStream::connect(path).unwrap();
        for l in lines {
            writeln!(s, "{l}").unwrap();
        }
        let mut r = BufReader::new(s.try_clone().unwrap());
        (0..answers)
            .map(|_| {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                serde_json::from_str(&line).unwrap()
            })
            .collect()
    }

    #[test]
    fn calls_go_through_the_sink() {
        let (_d, path, stub, _stop) = start(Stub::default());
        let mut c = Client::connect(&path).unwrap();
        let v = c.call_raw("run.list", json!({ "workspace": "w" })).unwrap();
        assert_eq!(v, json!({ "method": "run.list", "params": { "workspace": "w" } }));
        assert_eq!(stub.log.lock().unwrap()[0], "open true", "the peer is pinned");
    }

    #[test]
    fn bad_messages_get_protocol_errors() {
        let (_d, path, stub, _stop) = start(Stub::default());
        let a = raw(
            &path,
            &[
                "{not json",
                r#"[{"jsonrpc":"2.0","id":1,"method":"run.list"}]"#,
                r#"{"jsonrpc":"2.0","method":"run.list"}"#,
                r#"{"jsonrpc":"1.0","id":2,"method":"run.list"}"#,
                r#"{"jsonrpc":"2.0","id":3,"method":"run.teleport"}"#,
                r#"{"jsonrpc":"2.0","id":4,"method":"run.list","params":[1]}"#,
                r#"{"jsonrpc":"2.0","id":5,"method":"run.list"}"#,
            ],
            6,
        );
        let codes: Vec<i64> = a.iter().map(|r| r["error"]["code"].as_i64().unwrap_or(0)).collect();
        assert_eq!(codes, [rpc::PARSE_ERROR, rpc::INVALID_REQUEST, rpc::INVALID_REQUEST, rpc::METHOD_NOT_FOUND, rpc::INVALID_PARAMS, 0]);
        assert_eq!(a[3]["error"]["data"]["kind"], "not_found");
        assert_eq!(a[5]["result"]["params"], json!({}), "no params: an empty object");
        assert!(stub.log.lock().unwrap().contains(&"notification run.list".to_string()), "a notification gets no answer");
    }

    #[test]
    fn an_oversized_message_closes_the_connection() {
        let (_d, path, _stub, _stop) = start(Stub::default());
        let mut s = UnixStream::connect(&path).unwrap();
        let big = vec![b'x'; devdeck_api::MAX_MESSAGE + 100];
        let _ = s.write_all(&big);
        let mut r = BufReader::new(s.try_clone().unwrap());
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        let v: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["error"]["code"], rpc::INVALID_REQUEST);
        line.clear();
        assert_eq!(r.read_line(&mut line).unwrap(), 0, "closed");
    }

    #[test]
    fn an_expired_caller_is_answered_then_closed() {
        let (_d, path, _stub, _stop) = start(Stub::default());
        let mut c = Client::connect(&path).unwrap();
        match c.call_raw("session.whoami", json!({})) {
            Err(ClientError::Api(e)) => assert_eq!(e.kind, ErrorKind::Expired),
            other => panic!("{other:?}"),
        }
        assert!(matches!(c.call_raw("run.list", json!({})), Err(ClientError::Io(_))));
    }

    #[test]
    fn outside_connections_are_capped() {
        let (_d, path, stub, _stop) = start(Stub { outside: true, ..Default::default() });
        let held: Vec<Client> = (0..MAX_OUTSIDE).map(|_| Client::connect(&path).unwrap()).collect();
        // Each connection is opened on its thread: wait until all are.
        let opened = |s: &Stub| s.log.lock().unwrap().iter().filter(|l| l.starts_with("open")).count();
        while opened(&stub) < MAX_OUTSIDE {
            std::thread::yield_now();
        }
        let mut one_more = Client::connect(&path).unwrap();
        assert!(one_more.call_raw("run.list", json!({})).is_err());
        assert!(stub.log.lock().unwrap().iter().any(|l| l == "refused too many outside connections"));
        drop(held);
    }

    #[test]
    fn stopping_ends_accepting() {
        let (_d, path, _stub, mut stop) = start(Stub::default());
        stop.stop();
        // The accept thread ends; a connection may still queue but is never served.
        let served = UnixStream::connect(&path).ok().and_then(|s| {
            s.set_read_timeout(Some(std::time::Duration::from_millis(200))).ok()?;
            let mut s = s;
            writeln!(s, r#"{{"jsonrpc":"2.0","id":1,"method":"run.list"}}"#).ok()?;
            let mut line = String::new();
            BufReader::new(s).read_line(&mut line).ok().filter(|n| *n > 0)
        });
        assert!(served.is_none());
    }
}
