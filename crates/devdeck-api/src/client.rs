//! A blocking client: one connection, one call at a time. Messages without an id (events,
//! from a later level) are skipped.

use crate::methods::Method;
use crate::rpc::{ApiError, Request, Response};
use crate::{MAX_MESSAGE, SOCKET_ENV};
use serde_json::Value;
use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// The socket to use: `DEVDECK_SOCKET` (set in devdeck's shells), else the main devdeck's
/// in `$XDG_RUNTIME_DIR/devdeck/`. `env` reads a variable.
pub fn default_socket(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(s) = env(SOCKET_ENV).filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(s));
    }
    env("XDG_RUNTIME_DIR").filter(|s| !s.is_empty()).map(|d| PathBuf::from(d).join("devdeck").join("main.sock"))
}

/// Reads one message (a line) into `buf`, without its end of line. `Ok(false)` at the end
/// of the stream; an error past [`MAX_MESSAGE`] (nothing more is read of it).
pub fn read_message(r: &mut impl BufRead, buf: &mut Vec<u8>) -> io::Result<bool> {
    buf.clear();
    let n = io::Read::take(r, MAX_MESSAGE as u64 + 1).read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(false);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    } else if buf.len() > MAX_MESSAGE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message too long"));
    }
    Ok(true)
}

#[derive(Debug)]
pub enum ClientError {
    /// Couldn't reach devdeck, or the connection broke.
    Io(io::Error),
    /// devdeck answered with an error.
    Api(ApiError),
    /// devdeck's answer couldn't be read.
    Protocol(String),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "connection: {e}"),
            ClientError::Api(e) => write!(f, "{e}"),
            ClientError::Protocol(e) => write!(f, "protocol: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> Self {
        ClientError::Io(e)
    }
}

/// Notifications kept while calls wait, at most (the oldest go first).
pub const KEPT: usize = 1024;

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next: u64,
    /// Notifications that came while waiting for an answer, oldest first.
    pending: std::collections::VecDeque<Value>,
}

impl Client {
    pub fn connect(path: &Path) -> io::Result<Self> {
        let writer = UnixStream::connect(path)?;
        Ok(Self { reader: BufReader::new(writer.try_clone()?), writer, next: 1, pending: Default::default() })
    }

    /// How long to wait for an answer (none: as long as it takes); past it, the call
    /// fails as a broken connection.
    pub fn set_timeout(&mut self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.writer.set_read_timeout(timeout)
    }

    pub fn call<M: Method>(&mut self, params: &M::Params) -> Result<M::Result, ClientError> {
        let params = serde_json::to_value(params).map_err(|e| ClientError::Protocol(e.to_string()))?;
        let result = self.call_raw(M::NAME, params)?;
        serde_json::from_value(result).map_err(|e| ClientError::Protocol(format!("{}: {e}", M::NAME)))
    }

    /// The next notification (`{method, params}`): one kept while a call waited, else the
    /// next to come, within the read timeout ([`Client::set_timeout`]).
    pub fn next_notification(&mut self) -> Result<Value, ClientError> {
        if let Some(n) = self.pending.pop_front() {
            return Ok(n);
        }
        let mut buf = Vec::new();
        loop {
            if !read_message(&mut self.reader, &mut buf)? {
                return Err(ClientError::Io(io::Error::new(io::ErrorKind::UnexpectedEof, "devdeck closed the connection")));
            }
            let msg: Value = serde_json::from_slice(&buf).map_err(|e| ClientError::Protocol(e.to_string()))?;
            // No call is waiting: a stray answer isn't one to keep.
            if msg.get("id").is_none() {
                return Ok(msg);
            }
        }
    }

    /// A call by name, for clients that pass methods through (the CLI, MCP).
    pub fn call_raw(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next;
        self.next += 1;
        let mut line = serde_json::to_vec(&Request::new(id, method, params)).map_err(|e| ClientError::Protocol(e.to_string()))?;
        line.push(b'\n');
        if let Err(e) = self.writer.write_all(&line) {
            // devdeck may have refused the connection and closed it, saying why first.
            let mut buf = Vec::new();
            if read_message(&mut self.reader, &mut buf).unwrap_or(false)
                && let Ok(r) = serde_json::from_slice::<Response>(&buf)
                && let Some(err) = r.error
            {
                return Err(ClientError::Api(err.into()));
            }
            return Err(e.into());
        }
        let mut buf = Vec::new();
        loop {
            if !read_message(&mut self.reader, &mut buf)? {
                return Err(ClientError::Io(io::Error::new(io::ErrorKind::UnexpectedEof, "devdeck closed the connection")));
            }
            let msg: Value = serde_json::from_slice(&buf).map_err(|e| ClientError::Protocol(e.to_string()))?;
            // Not an answer: a notification (an event), kept for `next_notification`.
            if msg.get("id").is_none() {
                if self.pending.len() == KEPT {
                    self.pending.pop_front();
                }
                self.pending.push_back(msg);
                continue;
            }
            let r: Response = serde_json::from_value(msg).map_err(|e| ClientError::Protocol(e.to_string()))?;
            // A request devdeck couldn't read answers with a null id.
            if r.id != id && !r.id.is_null() {
                continue;
            }
            return match (r.result, r.error) {
                (_, Some(e)) => Err(ClientError::Api(e.into())),
                (Some(v), None) => Ok(v),
                (None, None) => Err(ClientError::Protocol("an answer with neither result nor error".into())),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::ErrorKind;
    use std::os::unix::net::UnixListener;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn socket_from_the_shell_else_the_main_one() {
        assert_eq!(default_socket(env(&[("DEVDECK_SOCKET", "/s"), ("XDG_RUNTIME_DIR", "/r")])), Some("/s".into()));
        assert_eq!(default_socket(env(&[("XDG_RUNTIME_DIR", "/r")])), Some("/r/devdeck/main.sock".into()));
        assert_eq!(default_socket(env(&[])), None);
    }

    #[test]
    fn messages_are_bounded() {
        let mut buf = Vec::new();
        let mut r = io::Cursor::new(b"{\"a\":1}\n{\"b\":2}".to_vec());
        assert!(read_message(&mut r, &mut buf).unwrap());
        assert_eq!(buf, b"{\"a\":1}");
        assert!(read_message(&mut r, &mut buf).unwrap(), "a last line without end");
        assert!(!read_message(&mut r, &mut buf).unwrap());
        let mut long = io::Cursor::new(vec![b'x'; MAX_MESSAGE + 10]);
        assert!(read_message(&mut long, &mut buf).is_err());
    }

    /// A server answering one line per request, after an event.
    fn serve(answer: &'static str) -> (crate::testing::Dir, PathBuf) {
        let dir = crate::testing::Dir::new();
        let path = dir.0.join("s.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut w = s;
            let mut buf = Vec::new();
            while read_message(&mut r, &mut buf).unwrap() {
                let req: Request = serde_json::from_slice(&buf).unwrap();
                let id = req.id.unwrap();
                writeln!(w, r#"{{"jsonrpc":"2.0","method":"event","params":{{}}}}"#).unwrap();
                writeln!(w, "{}", answer.replace("ID", &id.to_string())).unwrap();
            }
        });
        (dir, path)
    }

    #[test]
    fn calls_skip_events_and_read_errors() {
        let (_d, path) = serve(r#"{"jsonrpc":"2.0","id":ID,"result":{"on":true}}"#);
        let mut c = Client::connect(&path).unwrap();
        let r = c.call::<crate::methods::EditModeGet>(&crate::types::NoParams {}).unwrap();
        assert!(r.on);
        assert_eq!(c.next_notification().unwrap()["method"], "event", "the event that came first is kept");
        let (_d, path) = serve(r#"{"jsonrpc":"2.0","id":ID,"error":{"code":-32000,"message":"no","data":{"kind":"permission_denied"}}}"#);
        let mut c = Client::connect(&path).unwrap();
        match c.call_raw("run.get", serde_json::json!({})) {
            Err(ClientError::Api(e)) => assert_eq!(e.kind, ErrorKind::PermissionDenied),
            other => panic!("{other:?}"),
        }
    }
}
