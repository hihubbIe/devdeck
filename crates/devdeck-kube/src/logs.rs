//! A container's logs, streamed on the watches' runtime and shared clients (R7.5).
//!
//! - Lines carry the server's timestamp (`timestamps=true`), split off into
//!   [`LogLine::at`] in nanoseconds, and their level, both worked out here, off the UI
//!   thread. A line is cut at [`MAX_LINE`] bytes (marked) and decoded lossily: invalid
//!   UTF-8 or an endless line without a newline can't kill or bloat the stream.
//! - Lines are sent in batches, [`BATCH`] after the first line not sent yet, so a busy pod
//!   is one message per batch, not per line.
//! - A transport error (connection lost, reset, a server error) is a problem while the
//!   stream is tried again with the watches' backoff, from the last line shown: kube-core
//!   rounds `sinceTime` to the second, so the time asked is that line's truncated to the
//!   second, and what comes back older than it, or at exactly its time as many as were
//!   already shown, is dropped (a count: identical lines at one time can't be told apart
//!   by their text). A stream with no line for [`IDLE`] is resumed the same way, silently:
//!   kube has no read timeout and a log stream no server timeout, so a dead connection
//!   would otherwise hang forever.
//! - A refusal (401, 403, a 400) is sent once and not retried; the pod gone (404) is
//!   [`LogUpdate::Gone`]; a stream that ends on its own is [`LogUpdate::Ended`].

use crate::Target;
use crate::driver::{Backoff, WatchHandle, client, runtime, until_closed};
use crate::errors::describe;
use devdeck_core::runs::{Level, log_level};
use futures_lite::AsyncBufRead;
use futures_lite::io::AsyncBufReadExt;
use k8s_openapi::api::core::v1 as core;
use k8s_openapi::jiff::Timestamp;
use kube::api::LogParams;
use kube::{Api, Client};
use std::time::Duration;
use tokio::time::Instant;

/// How long after the first line not sent yet the lines are sent.
const BATCH: Duration = Duration::from_millis(50);
/// How long a stream may go without a line before it is resumed.
const IDLE: Duration = Duration::from_secs(5 * 60);
/// The bytes kept of a line (its timestamp included); the rest of it is skipped.
const MAX_LINE: usize = 4096;
/// What ends a line that was cut.
const CUT: &str = " …[cut]";
/// The lines read back at most when a stream started with [`LogFrom::Tail`] is resumed.
const RESUME_CAP: u32 = 1000;
/// What a logs stream does, for messages.
const DOING: &str = "read logs";
const NANOS: i64 = 1_000_000_000;

/// Where a logs stream starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LogFrom {
    /// The last `n` lines, then followed.
    Tail(u32),
    /// From the line seen last: `at` its server timestamp (epoch ns), `seen` how many lines
    /// at exactly `at` were already shown (skipped), at most `cap` lines read back.
    Since { at: i64, seen: u32, cap: u32 },
}

/// A line of logs.
#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    /// The server's timestamp, epoch nanoseconds; `None` when the line had none that
    /// parses (its text is then the whole line).
    pub at: Option<i64>,
    pub text: String,
    pub level: Level,
}

/// What a logs stream sends.
#[derive(Clone, Debug, PartialEq)]
pub enum LogUpdate {
    Lines(Vec<LogLine>),
    /// A transport problem while it retries (`Some`), or over (`None`, sent with the next
    /// line after one).
    Problem(Option<String>),
    /// The stream ended on its own: a followed container stopped, or a `previous` read is
    /// complete.
    Ended,
    /// The pod is gone (404).
    Gone,
    /// Refused; not retried.
    Refused(LogRefusal),
}

/// Why logs can't be read.
#[derive(Clone, Debug, PartialEq)]
pub enum LogRefusal {
    /// 403: "your account can't read logs in <env>".
    Forbidden(String),
    /// 400, the container hasn't started: its message in plain words.
    Waiting(String),
    /// 400, no previous run of the container to read.
    NoPrevious(String),
    /// 400, no such container in the pod.
    NoContainer(String),
    /// Anything else, described like the watches' errors.
    Other(String),
}

/// Streams `container`'s logs of `pod` in `target`'s namespace: followed unless `previous`
/// (the run before the last restart, read once). Ends when the handle or the receiver is
/// dropped.
pub fn logs(target: &Target, pod: &str, container: &str, from: LogFrom, previous: bool) -> (async_channel::Receiver<LogUpdate>, WatchHandle) {
    let (tx, rx) = async_channel::unbounded();
    let (target, pod, container) = (target.clone(), pod.to_owned(), container.to_owned());
    let sender = tx.clone();
    let task = runtime().spawn(until_closed(tx, async move {
        let mut out = Out::new(sender);
        let mut backoff = Backoff::new();
        let (client, _user) = loop {
            match client(&target).await {
                Ok(client) => break client,
                Err(e) => {
                    out.problem(describe(&e, &target, DOING));
                    backoff.wait().await;
                }
            }
        };
        stream(client, &target, &pod, &container, from, previous, out).await;
    }));
    (rx, WatchHandle { task: task.abort_handle() })
}

/// The last line shown: its timestamp, and how many lines at exactly that time were shown.
#[derive(Clone, Copy, Debug)]
struct Seen {
    at: i64,
    count: u32,
}

/// How one request ended.
enum End {
    /// The stream ended on its own.
    Eof,
    Gone,
    Refused(LogRefusal),
    /// A transport problem: tried again after the backoff.
    Failed(String),
    /// No line for [`IDLE`]: resumed at once, silently.
    Idle,
}

/// Sends the logs of `container` in `pod` through `client` until it ends, the pod goes or
/// it is refused.
async fn stream(client: Client, target: &Target, pod: &str, container: &str, from: LogFrom, previous: bool, mut out: Out) {
    let api: Api<core::Pod> = Api::namespaced(client, &target.namespace);
    let (tail, cap, mut seen) = match from {
        LogFrom::Tail(n) => (n, RESUME_CAP.max(n), None),
        LogFrom::Since { at, seen, cap } => (cap, cap, Some(Seen { at, count: seen })),
    };
    let mut backoff = Backoff::new();
    loop {
        let params = LogParams {
            container: Some(container.to_owned()),
            follow: !previous,
            previous,
            timestamps: true,
            tail_lines: Some(i64::from(if seen.is_some() { cap } else { tail })),
            since_time: seen.and_then(|s| Timestamp::from_second(s.at.div_euclid(NANOS)).ok()),
            ..LogParams::default()
        };
        let end = match tokio::time::timeout(IDLE, api.log_stream(pod, &params)).await {
            Err(_) => End::Idle,
            Ok(Err(e)) => refused(&e, target, pod, container),
            Ok(Ok(reader)) => {
                // Connected again: the problem is over, lines or not.
                out.clear_problem();
                read(Box::pin(reader), target, &mut seen, &mut out, &mut backoff).await
            }
        };
        out.flush();
        match end {
            End::Eof => return out.send(LogUpdate::Ended),
            End::Gone => return out.send(LogUpdate::Gone),
            End::Refused(refusal) => return out.send(LogUpdate::Refused(refusal)),
            End::Idle => {}
            End::Failed(problem) => {
                out.problem(problem);
                backoff.wait().await;
            }
        }
    }
}

/// Sends the lines of one request, those after `seen`, until it ends.
async fn read(reader: impl AsyncBufRead + Unpin, target: &Target, seen: &mut Option<Seen>, out: &mut Out, backoff: &mut Backoff) -> End {
    let mut lines = Lines::new(reader);
    // What this request sends again: older lines, and at exactly that time `count` lines.
    let mut skip = *seen;
    let mut idle_at = Instant::now() + IDLE;
    loop {
        let deadline = out.due.map_or(idle_at, |due| due.min(idle_at));
        let line = match tokio::time::timeout_at(deadline, lines.next()).await {
            Err(_) if out.due.is_some_and(|due| due <= Instant::now()) => {
                out.flush();
                continue;
            }
            Err(_) => return End::Idle,
            Ok(Err(e)) => return End::Failed(describe(&kube::Error::ReadEvents(e), target, DOING)),
            Ok(Ok(None)) => return End::Eof,
            Ok(Ok(Some(raw))) => parse(&raw),
        };
        idle_at = Instant::now() + IDLE;
        if let (Some(at), Some(s)) = (line.at, skip.as_mut()) {
            if at < s.at {
                continue;
            }
            if at == s.at && s.count > 0 {
                s.count -= 1;
                continue;
            }
            if at > s.at {
                skip = None;
            }
        }
        if let Some(at) = line.at {
            *seen = Some(match *seen {
                Some(s) if s.at == at => Seen { at, count: s.count + 1 },
                _ => Seen { at, count: 1 },
            });
        }
        *backoff = Backoff::new();
        out.line(line);
    }
}

/// What a refused request means: the pod gone, a refusal, or a server error tried again.
fn refused(e: &kube::Error, target: &Target, pod: &str, container: &str) -> End {
    let described = || describe(e, target, DOING);
    let kube::Error::Api(status) = e else { return End::Failed(described()) };
    let refusal = match status.code {
        404 => return End::Gone,
        403 => LogRefusal::Forbidden(described()),
        400 => {
            let message = status.message.to_ascii_lowercase();
            if let Some(i) = message.find("is waiting to start") {
                // `container "api" in pod "api-…" is waiting to start: ContainerCreating`
                let reason = status.message[i + "is waiting to start".len()..].trim_start_matches(':').trim();
                LogRefusal::Waiting(if reason.is_empty() { "waiting to start".into() } else { format!("waiting to start: {reason}") })
            } else if message.contains("previous terminated container") {
                // `previous terminated container "api" in pod "api-…" not found`
                LogRefusal::NoPrevious(format!("no previous run of {container} to read"))
            } else if message.contains("is not valid for pod") {
                // `container nope is not valid for pod api-…`
                LogRefusal::NoContainer(format!("no container {container} in {pod}"))
            } else {
                LogRefusal::Other(described())
            }
        }
        429 | 500.. => return End::Failed(described()),
        _ => LogRefusal::Other(described()),
    };
    End::Refused(refusal)
}

/// A line as the server sends it: its timestamp, a space, its text.
fn parse(raw: &str) -> LogLine {
    let stamped = raw.split_once(' ').and_then(|(stamp, text)| {
        let at = stamp.parse::<Timestamp>().ok()?;
        Some((i64::try_from(at.as_nanosecond()).ok()?, text))
    });
    let (at, text) = match stamped {
        Some((at, text)) => (Some(at), text),
        None => (None, raw),
    };
    LogLine { at, text: text.to_owned(), level: log_level(text) }
}

/// Lines of a body, each cut at [`MAX_LINE`] bytes and decoded lossily. Its state lives
/// here, not in [`Lines::next`]'s future: dropping that future between two lines' chunks
/// (a batch due) loses nothing.
struct Lines<R> {
    reader: R,
    line: Vec<u8>,
    cut: bool,
}

impl<R: AsyncBufRead + Unpin> Lines<R> {
    fn new(reader: R) -> Lines<R> {
        Lines { reader, line: Vec::new(), cut: false }
    }

    /// The next line, without its line ending; `None` at the end of the body.
    async fn next(&mut self) -> std::io::Result<Option<String>> {
        loop {
            let chunk = self.reader.fill_buf().await?;
            if chunk.is_empty() {
                // A last line without a newline is still a line.
                return Ok((!self.line.is_empty() || self.cut).then(|| self.take()));
            }
            let (end, used) = match chunk.iter().position(|&b| b == b'\n') {
                Some(i) => (true, i + 1),
                None => (false, chunk.len()),
            };
            let content = &chunk[..if end { used - 1 } else { used }];
            let room = MAX_LINE - self.line.len();
            self.line.extend_from_slice(&content[..content.len().min(room)]);
            self.cut |= content.len() > room;
            self.reader.consume(used);
            if end {
                return Ok(Some(self.take()));
            }
        }
    }

    fn take(&mut self) -> String {
        let mut bytes = std::mem::take(&mut self.line);
        if self.cut {
            // Not half a character where it was cut.
            if let Err(e) = std::str::from_utf8(&bytes)
                && e.error_len().is_none()
            {
                bytes.truncate(e.valid_up_to());
            }
        } else if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if std::mem::take(&mut self.cut) {
            text.push_str(CUT);
        }
        text
    }
}

/// The sending end of a logs stream: lines in batches, a problem when it changes.
struct Out {
    tx: async_channel::Sender<LogUpdate>,
    /// The problem shown, until a line clears it.
    problem: Option<String>,
    batch: Vec<LogLine>,
    /// When the batch is sent.
    due: Option<Instant>,
}

impl Out {
    fn new(tx: async_channel::Sender<LogUpdate>) -> Out {
        Out { tx, problem: None, batch: Vec::new(), due: None }
    }

    fn send(&self, update: LogUpdate) {
        let _ = self.tx.try_send(update);
    }

    fn problem(&mut self, problem: String) {
        if self.problem.as_ref() != Some(&problem) {
            self.send(LogUpdate::Problem(Some(problem.clone())));
            self.problem = Some(problem);
        }
    }

    fn line(&mut self, line: LogLine) {
        if self.batch.is_empty() {
            self.due = Some(Instant::now() + BATCH);
        }
        self.batch.push(line);
    }

    /// Says the problem is over, if there was one.
    fn clear_problem(&mut self) {
        if self.problem.take().is_some() {
            self.send(LogUpdate::Problem(None));
        }
    }

    /// Sends the batch, after clearing the problem it ends.
    fn flush(&mut self) {
        self.due = None;
        if self.batch.is_empty() {
            return;
        }
        if self.problem.take().is_some() {
            self.send(LogUpdate::Problem(None));
        }
        let batch = std::mem::take(&mut self.batch);
        self.send(LogUpdate::Lines(batch));
    }
}

#[cfg(test)]
mod tests;
