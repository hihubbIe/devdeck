//! What a logs view shows (R7.5): [`LogsBuffer`], the lines kept with what the level, the
//! pod and the filter show, kept up to date as lines come and go (nothing is recomputed
//! per frame); and [`LogsFeed`], one stream per pod of the service while the view is on
//! screen.
//!
//! - Pods there when the first list comes show their last [`FIRST_LINES`]; a pod that
//!   comes later (a rollout) joins with up to [`CATCH_UP`]; a pod that goes keeps its
//!   lines, a note says so, and its stream runs to its end ([`GONE_GRACE`] at most). A
//!   list emptied because the cluster forgot its pods (another driver, `clear`) isn't
//!   "gone".
//! - A stream starts only for a container that runs or has ended: a waiting one says its
//!   reason from the pod (ImagePullBackOff…) and nothing is asked of the cluster. A stream
//!   that ended (the container stopped) starts again when the container's restarts or
//!   state change.
//! - Hidden, every stream stops; shown again, each pod resumes from its last line (its
//!   server timestamp and how many lines at exactly that time were shown), so nothing is
//!   lost or doubled. The same when the env's target changes (the cluster's log
//!   generation) or its pod watch comes back after a problem.
//! - The first lines of streams started together are merged by time (for at most
//!   [`MERGE_WAIT`]); later lines show in the order they come. Updates are drained at
//!   most once per [`DRAIN_EVERY`], one render for all.
//! - Paused, or not following with the buffer full, lines wait in a queue of their own
//!   (capped and counted), shown on resume or once following again: what's on screen
//!   isn't pushed out while someone reads it.
//! - Reading back ([`LogsFeed::earlier`]): one read of everything the node serves per pod,
//!   cut where the oldest line shown is (its time and how many lines at that time are
//!   shown), the older lines of every pod merged by time and put above
//!   ([`LogsBuffer::prepend`]) within the caps. A pod read once is at its start until its
//!   restarts or the generation change.

use super::logs_view::{MinLevel, ServiceLogsView};
use crate::cluster::{Cluster, LogFrom, LogLine, LogRefusal, LogStream, LogUpdate};
use devdeck_core::cluster::Pod;
use devdeck_core::runs::Level;
use gpui::{Context, Entity, SharedString, Task};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;

/// Lines of each pod shown when the view opens.
pub const FIRST_LINES: u32 = 200;
/// Most lines a pod reads back when it joins or resumes.
pub const CATCH_UP: u32 = 1000;
/// Lines kept for the whole view.
pub const MAX_LINES: usize = 100_000;
/// Bytes of text kept for the whole view.
pub const MAX_BYTES: usize = 64 << 20;
/// The absolute index of the first line: lines read back go above it, indices going down.
pub const FIRST_INDEX: u64 = 1 << 40;
/// How long a gone pod's stream may run to its end.
pub const GONE_GRACE: Duration = Duration::from_secs(30);
/// How long the first lines of streams started together wait for each other.
pub const MERGE_WAIT: Duration = Duration::from_secs(2);
/// How often updates are drained into the view (about a frame).
pub const DRAIN_EVERY: Duration = Duration::from_millis(16);
/// How long after a stream ended (or was refused as not started) while its container
/// still runs it's asked again: a server or proxy can end a stream cleanly.
pub const ENDED_RETRY: Duration = Duration::from_secs(5);

/// What a row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A line of a pod's current run.
    Line,
    /// A note of devdeck's ("pod … gone").
    Note,
    /// A line of a pod's run before its last restart.
    Previous,
}

/// One line shown: a pod's, or a note of devdeck's.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The server's timestamp (epoch ns).
    pub at: Option<i64>,
    /// The pod's full name.
    pub pod: SharedString,
    pub text: SharedString,
    pub level: Level,
    pub kind: Kind,
}

impl Entry {
    pub fn line(pod: &SharedString, l: LogLine) -> Self {
        Self { at: l.at, pod: pod.clone(), text: l.text.into(), level: l.level, kind: Kind::Line }
    }

    fn previous(pod: &SharedString, l: LogLine) -> Self {
        Self { kind: Kind::Previous, ..Self::line(pod, l) }
    }

    fn note(pod: &SharedString, text: String) -> Self {
        Self { at: None, pod: pod.clone(), text: text.into(), level: Level::Plain, kind: Kind::Note }
    }

    pub fn is_note(&self) -> bool {
        self.kind == Kind::Note
    }
}

/// A pod's name as rows show it: its last part (`graphservice-5d9f-x2k8q` → `x2k8q`).
pub fn short(pod: &str) -> &str {
    pod.rsplit('-').next().unwrap_or(pod)
}

/// What [`LogsBuffer::prepend`] put above.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Prepended {
    /// Lines put above.
    pub lines: usize,
    /// Of them, the lines shown (the first rows now).
    pub rows: usize,
    /// Some didn't fit (the oldest were left out).
    pub full: bool,
}

/// The lines kept, by absolute index (the first line ever is [`FIRST_INDEX`], lines put
/// above go down from it), and what the level, the pod picked and the filter show, kept up
/// to date line by line.
pub struct LogsBuffer {
    entries: VecDeque<Entry>,
    /// Each entry's length in characters (finding the longest again or wrapping costs no
    /// counting).
    chars: VecDeque<usize>,
    /// The absolute index of `entries[0]`.
    first: u64,
    /// Lines dropped for room.
    dropped: u64,
    bytes: usize,
    min: MinLevel,
    pod: Option<SharedString>,
    /// The filter's characters, lowercase; empty: no filter.
    query: Vec<char>,
    /// The filter is all ASCII (matched on bytes).
    ascii: bool,
    /// Absolute indices of the lines shown, in order.
    rows: VecDeque<u64>,
    /// The longest line shown, by characters.
    longest: Option<(u64, usize)>,
    max_lines: usize,
    max_bytes: usize,
    /// Moves when the rows shown change other than lines added at the end or dropped from
    /// the front (a filter, lines put above): what's worked out per row starts again.
    reshaped: u64,
}

impl Default for LogsBuffer {
    fn default() -> Self {
        Self::new(MAX_LINES, MAX_BYTES)
    }
}

impl LogsBuffer {
    pub fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            chars: VecDeque::new(),
            first: FIRST_INDEX,
            dropped: 0,
            bytes: 0,
            min: MinLevel::All,
            pod: None,
            query: Vec::new(),
            ascii: true,
            rows: VecDeque::new(),
            longest: None,
            max_lines: max_lines.max(1),
            max_bytes,
            reshaped: 0,
        }
    }

    fn shows(&self, e: &Entry) -> bool {
        // A note about no pod in particular shows whatever pod is picked.
        let pod = e.pod.is_empty() || self.pod.as_ref().is_none_or(|p| *p == e.pod);
        pod && (e.is_note() || super::logs_view::shown(e.level, self.min)) && self.holds(&e.text)
    }

    /// Whether `text` holds the filter (always without one).
    fn holds(&self, text: &str) -> bool {
        if self.query.is_empty() {
            return true;
        }
        if self.ascii && text.is_ascii() {
            let (t, n) = (text.as_bytes(), self.query.len());
            return t.windows(n).any(|w| w.iter().zip(&self.query).all(|(b, q)| b.to_ascii_lowercase() == *q as u8));
        }
        holds(text, &self.query)
    }

    /// Whether `e` fits without pushing a line out.
    pub fn fits(&self, e: &Entry) -> bool {
        self.entries.is_empty() || (self.entries.len() < self.max_lines && self.bytes + e.text.len() <= self.max_bytes)
    }

    pub fn push(&mut self, e: Entry) {
        let abs = self.first + self.entries.len() as u64;
        self.bytes += e.text.len();
        let chars = e.text.chars().count();
        if self.shows(&e) {
            self.rows.push_back(abs);
            if self.longest.is_none_or(|(_, n)| chars > n) {
                self.longest = Some((abs, chars));
            }
        }
        self.entries.push_back(e);
        self.chars.push_back(chars);
        let mut lost_longest = false;
        while self.entries.len() > self.max_lines || (self.bytes > self.max_bytes && self.entries.len() > 1) {
            let Some(gone) = self.entries.pop_front() else { break };
            self.bytes -= gone.text.len();
            self.chars.pop_front();
            if self.longest.is_some_and(|(i, _)| i == self.first) {
                lost_longest = true;
            }
            self.first += 1;
            self.dropped += 1;
        }
        while self.rows.front().is_some_and(|i| *i < self.first) {
            self.rows.pop_front();
        }
        if lost_longest {
            self.find_longest();
        }
    }

    /// Puts `lines` (oldest first) above the first line, as many as fit within the caps:
    /// the newest of them, nothing pushed out.
    pub fn prepend(&mut self, lines: Vec<Entry>) -> Prepended {
        let mut room = (self.max_lines.saturating_sub(self.entries.len()), self.max_bytes.saturating_sub(self.bytes));
        let mut from = lines.len();
        while from > 0 {
            let len = lines[from - 1].text.len();
            if room.0 == 0 || len > room.1 {
                break;
            }
            room = (room.0 - 1, room.1 - len);
            from -= 1;
        }
        let full = from > 0;
        let kept: Vec<Entry> = lines.into_iter().skip(from).collect();
        if kept.is_empty() {
            return Prepended { full, ..Default::default() };
        }
        let first = self.first - kept.len() as u64;
        let mut shown = Vec::new();
        let mut chars = Vec::with_capacity(kept.len());
        for (i, e) in kept.iter().enumerate() {
            let n = e.text.chars().count();
            chars.push(n);
            if self.shows(e) {
                let abs = first + i as u64;
                shown.push(abs);
                if self.longest.is_none_or(|(_, m)| n > m) {
                    self.longest = Some((abs, n));
                }
            }
        }
        let added = Prepended { lines: kept.len(), rows: shown.len(), full };
        for (e, n) in kept.into_iter().zip(chars).rev() {
            self.bytes += e.text.len();
            self.entries.push_front(e);
            self.chars.push_front(n);
        }
        for abs in shown.into_iter().rev() {
            self.rows.push_front(abs);
        }
        self.first = first;
        self.reshaped += 1;
        added
    }

    fn find_longest(&mut self) {
        self.longest = self.rows.iter().map(|i| (*i, self.chars[(*i - self.first) as usize])).max_by_key(|(_, n)| *n);
    }

    fn refilter(&mut self) {
        self.reshaped += 1;
        self.rows = (0..self.entries.len()).filter(|i| self.shows(&self.entries[*i])).map(|i| self.first + i as u64).collect();
        self.find_longest();
    }

    pub fn set_min(&mut self, min: MinLevel) {
        self.min = min;
        self.refilter();
    }

    pub fn set_pod(&mut self, pod: Option<SharedString>) {
        self.pod = pod;
        self.refilter();
    }

    pub fn pod(&self) -> Option<&SharedString> {
        self.pod.as_ref()
    }

    /// Shows only the lines holding `query`, ignoring case (empty: every line).
    pub fn set_query(&mut self, query: &str) {
        self.query = query.chars().flat_map(char::to_lowercase).collect();
        self.ascii = self.query.iter().all(char::is_ascii);
        self.refilter();
    }

    /// Every line goes (another container); not counted as dropped.
    pub fn clear(&mut self) {
        self.reshaped += 1;
        self.first += self.entries.len() as u64;
        self.entries.clear();
        self.chars.clear();
        self.bytes = 0;
        self.rows.clear();
        self.longest = None;
    }

    /// Moves when the rows shown changed other than at their ends (see the field).
    pub fn reshaped(&self) -> u64 {
        self.reshaped
    }

    /// Lines kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether no other line fits without pushing one out.
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.max_lines || self.bytes >= self.max_bytes
    }

    /// The most lines kept.
    pub fn max_lines(&self) -> usize {
        self.max_lines
    }

    /// Lines dropped to stay within the caps.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The absolute index of the first line kept.
    pub fn first_line(&self) -> u64 {
        self.first
    }

    /// Lines shown.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The line shown at `row`.
    pub fn row(&self, row: usize) -> Option<&Entry> {
        self.line(*self.rows.get(row)?)
    }

    /// The absolute index of the line shown at `row`.
    pub fn row_line(&self, row: usize) -> Option<u64> {
        self.rows.get(row).copied()
    }

    /// The row showing the line `abs`, if it's shown.
    pub fn row_of(&self, abs: u64) -> Option<usize> {
        self.rows.binary_search(&abs).ok()
    }

    /// The line `abs`, if it's kept.
    pub fn line(&self, abs: u64) -> Option<&Entry> {
        self.entries.get(usize::try_from(abs.checked_sub(self.first)?).ok()?)
    }

    /// The length in characters of the line `abs` (0 if it's not kept).
    pub fn chars_of(&self, abs: u64) -> usize {
        abs.checked_sub(self.first).and_then(|i| self.chars.get(i as usize)).copied().unwrap_or(0)
    }

    /// The lines shown after the line `after` (every one for `None`), in order.
    pub fn rows_after(&self, after: Option<u64>) -> impl Iterator<Item = u64> + '_ {
        let from = after.map_or(0, |a| self.rows.partition_point(|r| *r <= a));
        self.rows.range(from..).copied()
    }

    /// The row of the longest line shown (sizes the sideways scroll).
    pub fn longest_row(&self) -> Option<usize> {
        self.row_of(self.longest?.0)
    }

    /// The filter's characters, lowercase.
    pub fn query(&self) -> &[char] {
        &self.query
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }
}

/// What a pod's stream is doing.
#[derive(Default)]
struct PodFeed {
    name: SharedString,
    /// The running stream and its id (updates of an older one are ignored).
    stream: Option<(u64, Task<()>)>,
    /// The last line shown: its server time and how many lines had exactly that time.
    last: Option<(i64, u32)>,
    /// The container's restarts and state, as the pod said last.
    seen: (u32, String),
    /// Its stream ended with the container in this state: started again on a change.
    ended: Option<(u32, String)>,
    /// Why its container isn't running yet (ImagePullBackOff…).
    waiting: Option<String>,
    /// Refused (403, 401): not asked again until the generation changes.
    refused: Option<LogRefusal>,
    /// Asks again after [`ENDED_RETRY`] if the container still runs.
    retry: Option<Task<()>>,
    /// A problem while its stream retries.
    problem: Option<String>,
    /// Left the list: its stream runs to its end, then it's forgotten.
    gone: bool,
    /// Forgets it once gone for [`GONE_GRACE`].
    grace: Option<Task<()>>,
    /// Its previous run being read, and its lines so far (shown together at its end).
    previous: Option<(u64, Task<()>, Vec<Entry>)>,
    /// Read back to its start with these restarts and this generation.
    start: Option<(u32, (u64, u64))>,
}

impl PodFeed {
    /// Its container runs or ran, and reading isn't refused.
    fn readable(&self) -> bool {
        !self.gone && self.refused.is_none() && (self.seen.1.starts_with("running") || self.seen.1.starts_with("terminated"))
    }
}

/// A read back under way: what must still hold when it ends for its lines to be used.
struct Reading {
    id: u64,
    _task: Task<()>,
    container: Option<String>,
    generation: (u64, u64),
    /// Each pod read, its restarts when the read started, and its oldest line shown (its
    /// time, how many lines at that time are shown).
    pods: Vec<(SharedString, u32, (i64, u32))>,
}

/// How the last read back ended, when it says more than "load earlier lines".
#[derive(Clone, Debug, PartialEq)]
pub enum BackEnd {
    /// Some lines didn't fit: the newest of them were kept.
    Full,
    /// These pods' lines couldn't be lined up with those shown (short names).
    Unaligned(Vec<String>),
    Failed(String),
}

/// What the top row says.
#[derive(Clone, Debug, PartialEq)]
pub enum Back {
    /// Earlier lines can be read.
    Idle,
    Reading,
    /// Every pod was read to its start.
    Start,
    Ended(BackEnd),
}

/// What one pod's read back brought: its lines, or why not.
pub type ReadResult = (SharedString, Result<Vec<LogLine>, String>);

/// Lines held while the first lines of streams started together arrive.
struct Merge {
    /// Every pod of the merge: all their lines are held until it's over.
    members: BTreeSet<SharedString>,
    /// The members whose first lines haven't come yet.
    waiting: BTreeSet<SharedString>,
    lines: Vec<Entry>,
    _deadline: Task<()>,
}

/// The service's pods' streams, while the view is on screen.
pub struct LogsFeed {
    pub env: String,
    pub name: String,
    /// The container picked; `None`: each pod's main one.
    pub container: Option<String>,
    pods: BTreeMap<SharedString, PodFeed>,
    /// The first list of pods came.
    listed: bool,
    generation: (u64, u64),
    pods_problem: bool,
    visible: bool,
    queue: Vec<(SharedString, u64, LogUpdate)>,
    drain: Option<Task<()>>,
    merge: Option<Merge>,
    next_id: u64,
    pub buffer: LogsBuffer,
    pub paused: bool,
    /// The view follows new lines: when not, a full buffer keeps what's shown and new
    /// lines wait.
    following: bool,
    /// Lines that came while paused, or while not following with the buffer full.
    held: VecDeque<Entry>,
    /// Lines that came meanwhile and didn't fit.
    held_dropped: u64,
    reading: Option<Reading>,
    /// How the last read back ended; forgotten when the lines go or the generation changes.
    last: Option<BackEnd>,
}

impl LogsFeed {
    pub fn new(env: String, name: String) -> Self {
        Self {
            env,
            name,
            container: None,
            pods: BTreeMap::new(),
            listed: false,
            generation: (0, 0),
            pods_problem: false,
            visible: false,
            queue: Vec::new(),
            drain: None,
            merge: None,
            next_id: 0,
            buffer: LogsBuffer::default(),
            paused: false,
            following: true,
            held: VecDeque::new(),
            held_dropped: 0,
            reading: None,
            last: None,
        }
    }

    /// Streams running (tests, the leak run).
    pub fn streams(&self) -> usize {
        self.pods.values().filter(|p| p.stream.is_some()).count()
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Lines waiting to be shown (paused, or not following with the buffer full).
    pub fn held(&self) -> u64 {
        self.held.len() as u64 + self.held_dropped
    }

    /// Pods waiting to start, by reason (`ContainerCreating` → 2).
    pub fn waiting(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for p in self.pods.values().filter(|p| !p.gone) {
            if let Some(w) = &p.waiting {
                *out.entry(w.clone()).or_default() += 1;
            }
        }
        out
    }

    /// Why the logs can't be read at all: every pod asked refused for the same reason (a
    /// 403); pods waiting to start weren't asked.
    pub fn refused(&self) -> Option<String> {
        let mut reasons = self.pods.values().filter(|p| !p.gone && p.waiting.is_none()).map(|p| match &p.refused {
            Some(LogRefusal::Forbidden(why)) => Some(why.clone()),
            _ => None,
        });
        let first = reasons.next()??;
        reasons.all(|r| r.as_deref() == Some(first.as_str())).then_some(first)
    }

    /// What's wrong with each pod's stream now (a problem, a refusal), by pod.
    pub fn problems(&self) -> Vec<(SharedString, String)> {
        self.pods
            .values()
            .filter_map(|p| {
                let why = p.problem.clone().or_else(|| p.refused.as_ref().map(refusal_text))?;
                Some((p.name.clone(), why))
            })
            .collect()
    }

    /// The pods known, alphabetical, and whether each is gone.
    pub fn pods(&self) -> Vec<(SharedString, bool)> {
        self.pods.values().map(|p| (p.name.clone(), p.gone)).collect()
    }

    /// The container streamed for `pod`: the one picked, else its main one (the one named
    /// like the deployment, else the first that isn't an init container).
    pub(crate) fn container_of(&self, pod: &Pod) -> Option<String> {
        if let Some(c) = &self.container {
            return Some(c.clone());
        }
        let main = pod.containers.iter().find(|c| !c.init && c.name == self.name).or_else(|| pod.containers.iter().find(|c| !c.init))?;
        Some(main.name.clone())
    }

    /// On screen or not: hidden, every stream stops; shown, they resume.
    pub fn set_visible(&mut self, visible: bool, cluster: &Entity<Cluster>, cx: &mut Context<ServiceLogsView>) {
        self.visible = visible;
        if !visible {
            self.stop_all();
        }
        self.sync(cluster, cx);
    }

    fn stop_all(&mut self) {
        // Its result would be dropped anyway.
        self.reading = None;
        for p in self.pods.values_mut() {
            p.stream = None;
            p.previous = None;
            p.retry = None;
        }
        // Their last line counts them as shown: show them, or a resume would skip them.
        self.flush_merge();
    }

    /// Picks another container (`None`: each pod's main one): the lines go, and every pod
    /// starts again with its last lines.
    pub fn set_container(&mut self, container: Option<String>, cluster: &Entity<Cluster>, cx: &mut Context<ServiceLogsView>) {
        self.container = container;
        self.buffer.clear();
        self.held.clear();
        self.held_dropped = 0;
        self.merge = None;
        self.reading = None;
        self.last = None;
        for p in self.pods.values_mut() {
            *p = PodFeed { name: p.name.clone(), gone: p.gone, ..Default::default() };
        }
        self.pods.retain(|_, p| !p.gone);
        self.sync(cluster, cx);
        cx.notify();
    }

    /// Keeps the streams in line with the service's pods, the cluster's generation and
    /// whether the view is on screen.
    pub fn sync(&mut self, cluster: &Entity<Cluster>, cx: &mut Context<ServiceLogsView>) {
        let c = cluster.read(cx);
        let generation = c.log_generation(&self.env);
        let mut resume = false;
        if generation != self.generation {
            self.generation = generation;
            resume = true;
            self.reading = None;
            self.last = None;
            for p in self.pods.values_mut() {
                p.refused = None;
            }
        }
        let problem = c.pods_problem(&self.env).is_some();
        if self.pods_problem && !problem {
            resume = true;
        }
        self.pods_problem = problem;
        if !self.visible {
            return;
        }
        let pods: Vec<Pod> = match c.pods_state(&self.env, &self.name) {
            devdeck_core::cluster::Remote::Ready(pods) => pods.to_vec(),
            // Loading or forgotten: nothing is gone.
            _ => return,
        };
        // A container picked that the pods don't have any more (a rollout): back to the main one.
        if let Some(picked) = &self.container
            && !pods.is_empty()
            && !pods.iter().any(|p| p.containers.iter().any(|c| &c.name == picked))
        {
            let note = format!("container {picked} isn't in the pods any more: showing each pod's main container");
            self.deliver(Entry::note(&SharedString::default(), note));
            self.container = None;
            resume = true;
        }
        let first = !self.listed;
        self.listed = true;
        let names: BTreeSet<SharedString> = pods.iter().map(|p| SharedString::from(p.name.clone())).collect();
        let mut start = Vec::new();
        for pod in &pods {
            let name: SharedString = pod.name.clone().into();
            let container = self.container_of(pod);
            let status = container.as_ref().and_then(|c| pod.containers.iter().find(|s| &s.name == c));
            let new = !self.pods.contains_key(&name);
            let feed = self.pods.entry(name.clone()).or_insert_with(|| PodFeed { name: name.clone(), ..Default::default() });
            feed.gone = false;
            feed.grace = None;
            let Some(status) = status else {
                feed.waiting = Some("no such container".into());
                continue;
            };
            feed.seen = (status.restarts, status.state.clone());
            feed.waiting = status.state.strip_prefix("waiting: ").map(str::to_string).or_else(|| (status.state == "waiting").then(|| "waiting".to_string()));
            let startable = status.state.starts_with("running") || status.state.starts_with("terminated");
            if !startable || feed.refused.is_some() {
                feed.stream = None;
                continue;
            }
            if feed.ended.as_ref().is_some_and(|e| *e == feed.seen) {
                continue;
            }
            if feed.stream.is_none() || resume {
                let lines = if first || !new { FIRST_LINES } else { CATCH_UP };
                start.push((name, container.unwrap_or_default(), lines));
            }
        }
        // Pods that left the list: their stream runs to its end, a note says they went.
        let gone: Vec<SharedString> = self.pods.keys().filter(|n| !names.contains(*n)).cloned().collect();
        for name in gone {
            let Some(feed) = self.pods.get_mut(&name) else { continue };
            if feed.gone {
                continue;
            }
            feed.gone = true;
            feed.waiting = None;
            if feed.stream.is_some() {
                let n = name.clone();
                feed.grace = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(GONE_GRACE).await;
                    let _ = this.update(cx, |v, cx| {
                        if v.feed.pods.get(&n).is_some_and(|p| p.gone) {
                            v.feed.pods.remove(&n);
                            cx.notify();
                        }
                    });
                }));
            } else {
                self.pods.remove(&name);
            }
            let text = format!("pod {} gone", short(&name));
            self.deliver(Entry::note(&name, text));
        }
        for (name, container, lines) in start {
            self.start(&name, &container, lines, cluster, cx);
        }
    }

    fn start(&mut self, name: &SharedString, container: &str, lines: u32, cluster: &Entity<Cluster>, cx: &mut Context<ServiceLogsView>) {
        let Some(feed) = self.pods.get(name) else { return };
        let from = match feed.last {
            Some((at, seen)) => LogFrom::Since { at, seen, cap: CATCH_UP },
            None => LogFrom::Tail(lines),
        };
        let (env, pod) = (self.env.clone(), name.to_string());
        let started = cluster.update(cx, |c, cx| c.stream_logs(&env, &pod, container, from, false, cx));
        self.next_id += 1;
        let id = self.next_id;
        let Some(feed) = self.pods.get_mut(name) else { return };
        match started {
            Ok(stream) => {
                feed.ended = None;
                feed.problem = None;
                feed.stream = Some((id, read(stream, name.clone(), id, cx)));
                let merge = self.merge.get_or_insert_with(|| Merge {
                    members: BTreeSet::new(),
                    waiting: BTreeSet::new(),
                    lines: Vec::new(),
                    _deadline: cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(MERGE_WAIT).await;
                        let _ = this.update(cx, |v, cx| {
                            v.feed.flush_merge();
                            cx.notify();
                        });
                    }),
                });
                merge.members.insert(name.clone());
                merge.waiting.insert(name.clone());
            }
            Err(why) => feed.problem = Some(why),
        }
    }

    /// Reads `pod`'s run before its last restart once, between two notes.
    pub fn read_previous(&mut self, pod: &SharedString, cluster: &Entity<Cluster>, cx: &mut Context<ServiceLogsView>) -> Result<(), String> {
        let container = {
            let c = cluster.read(cx);
            let p = c.pods(&self.env, &self.name).iter().find(|p| p.name == pod.as_ref()).ok_or_else(|| format!("pod {} is gone", short(pod)))?;
            let container = self.container_of(p).ok_or("the pod has no container")?;
            if p.containers.iter().find(|s| s.name == container).is_none_or(|s| s.restarts == 0) {
                return Err(format!("{container} of pod {} hasn't restarted", short(pod)));
            }
            container
        };
        if self.pods.get(pod).is_some_and(|f| f.previous.is_some()) {
            return Err(format!("the previous run of {} is being read", short(pod)));
        }
        let env = self.env.clone();
        let stream = cluster.update(cx, |c, cx| c.stream_logs(&env, pod, &container, LogFrom::Tail(FIRST_LINES), true, cx))?;
        self.next_id += 1;
        let id = self.next_id;
        let task = read(stream, pod.clone(), id, cx);
        let Some(feed) = self.pods.get_mut(pod) else { return Err(format!("pod {} is gone", short(pod))) };
        let opening = Entry::note(pod, format!("previous run of {} ({container}):", short(pod)));
        feed.previous = Some((id, task, vec![opening]));
        Ok(())
    }

    /// An update of a stream: queued, drained at most once a frame.
    fn receive(&mut self, pod: SharedString, id: u64, update: LogUpdate, cx: &mut Context<ServiceLogsView>) {
        self.queue.push((pod, id, update));
        if self.drain.is_none() {
            self.drain = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DRAIN_EVERY).await;
                let _ = this.update(cx, |v, cx| {
                    v.feed.drain = None;
                    v.feed.drain_queue(cx);
                    v.after_lines(cx);
                });
            }));
        }
    }

    fn drain_queue(&mut self, cx: &mut Context<ServiceLogsView>) {
        for (pod, id, update) in std::mem::take(&mut self.queue) {
            self.apply(pod, id, update, cx);
        }
    }

    /// A stream ended (or wasn't started yet) with its container as the pod said last:
    /// it starts again when the container changes, or after [`ENDED_RETRY`] if it still
    /// runs.
    fn ended(&mut self, pod: &SharedString, cx: &mut Context<ServiceLogsView>) {
        let Some(feed) = self.pods.get_mut(pod) else { return };
        feed.stream = None;
        feed.ended = Some(feed.seen.clone());
        let p = pod.clone();
        feed.retry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ENDED_RETRY).await;
            let _ = this.update(cx, |v, cx| {
                if let Some(feed) = v.feed.pods.get_mut(&p)
                    && feed.ended.is_some()
                    && feed.seen.1.starts_with("running")
                {
                    feed.ended = None;
                    v.resync(cx);
                }
            });
        }));
        self.unmerge(pod);
    }

    fn apply(&mut self, pod: SharedString, id: u64, update: LogUpdate, cx: &mut Context<ServiceLogsView>) {
        let Some(feed) = self.pods.get_mut(&pod) else { return };
        let live = feed.stream.as_ref().is_some_and(|(i, _)| *i == id);
        if let Some((i, _, lines)) = &mut feed.previous
            && *i == id
        {
            // Held, then shown at once between its notes: live lines don't come in between.
            let end = match update {
                LogUpdate::Lines(l) => {
                    lines.extend(l.into_iter().map(|l| Entry::previous(&pod, l)));
                    return;
                }
                LogUpdate::Problem(_) => return,
                LogUpdate::Ended | LogUpdate::Gone => format!("end of the previous run of {}", short(&pod)),
                LogUpdate::Refused(r) => format!("previous run of {}: {}", short(&pod), refusal_text(&r)),
            };
            let Some((_, _, mut lines)) = feed.previous.take() else { return };
            lines.push(Entry::note(&pod, end));
            for e in lines {
                self.deliver(e);
            }
            return;
        }
        if !live {
            return;
        }
        match update {
            LogUpdate::Lines(lines) => {
                for l in &lines {
                    if let Some(at) = l.at {
                        feed.last = match feed.last {
                            Some((last, seen)) if last == at => Some((at, seen + 1)),
                            _ => Some((at, 1)),
                        };
                    }
                }
                let entries: Vec<Entry> = lines.into_iter().map(|l| Entry::line(&pod, l)).collect();
                match &mut self.merge {
                    Some(m) if m.members.contains(&pod) => {
                        m.lines.extend(entries);
                        m.waiting.remove(&pod);
                        if m.waiting.is_empty() {
                            self.flush_merge();
                        }
                    }
                    _ => {
                        for e in entries {
                            self.deliver(e);
                        }
                    }
                }
            }
            LogUpdate::Problem(p) => feed.problem = p,
            LogUpdate::Ended | LogUpdate::Gone => self.ended(&pod, cx),
            // Not started yet, or no such container: asked again when the container changes.
            LogUpdate::Refused(LogRefusal::Waiting(_) | LogRefusal::NoContainer(_) | LogRefusal::NoPrevious(_)) => self.ended(&pod, cx),
            LogUpdate::Refused(r) => {
                feed.stream = None;
                feed.refused = Some(r);
                self.unmerge(&pod);
            }
        }
    }

    fn unmerge(&mut self, pod: &SharedString) {
        if let Some(m) = &mut self.merge {
            m.waiting.remove(pod);
            if m.waiting.is_empty() {
                self.flush_merge();
            }
        }
    }

    fn flush_merge(&mut self) {
        let Some(mut m) = self.merge.take() else { return };
        // Stable: a pod's own lines keep their order.
        m.lines.sort_by_key(|e| e.at.unwrap_or(i64::MAX));
        for e in m.lines {
            self.deliver(e);
        }
    }

    fn deliver(&mut self, e: Entry) {
        // Behind those already waiting; not pushing out what's read while not following.
        if self.paused || !self.held.is_empty() || (!self.following && !self.buffer.fits(&e)) {
            self.held.push_back(e);
            if self.held.len() > MAX_LINES {
                self.held.pop_front();
                self.held_dropped += 1;
            }
        } else {
            self.buffer.push(e);
        }
    }

    /// Shows the lines waiting that can be: all of them when following, else those that
    /// fit; none while paused.
    fn flush_held(&mut self) {
        if self.paused {
            return;
        }
        while let Some(e) = self.held.front() {
            if !self.following && !self.buffer.fits(e) {
                break;
            }
            if let Some(e) = self.held.pop_front() {
                self.buffer.push(e);
            }
        }
        if self.held.is_empty() {
            self.held_dropped = 0;
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.flush_held();
    }

    /// Whether the view follows new lines (see [`LogsFeed::held`]).
    pub fn set_following(&mut self, following: bool) {
        self.following = following;
        self.flush_held();
    }

    pub fn is_following(&self) -> bool {
        self.following
    }

    fn at_start(&self, p: &PodFeed) -> bool {
        p.start == Some((p.seen.0, self.generation))
    }

    /// What the top row says.
    pub fn back(&self) -> Back {
        if self.reading.is_some() {
            return Back::Reading;
        }
        if let Some(end) = &self.last {
            return Back::Ended(end.clone());
        }
        if self.pods.values().filter(|p| p.readable()).all(|p| self.at_start(p)) {
            return Back::Start;
        }
        Back::Idle
    }

    /// Why earlier lines can't be read now, if they can't.
    pub fn earlier_refusal(&self) -> Option<String> {
        match self.back() {
            Back::Idle | Back::Ended(BackEnd::Failed(_)) => None,
            Back::Reading => Some("earlier lines are being read".into()),
            Back::Ended(BackEnd::Full) => Some(format!("full: {} lines kept", thousands(self.buffer.max_lines()))),
            Back::Start | Back::Ended(BackEnd::Unaligned(_)) => Some("these are the earliest lines the cluster serves".into()),
        }
    }

    /// Each pod's oldest line shown, merging or waiting (notes and previous runs left out):
    /// its time and how many of its lines have exactly that time.
    fn oldest(&self) -> BTreeMap<SharedString, (i64, u32)> {
        let mut out: BTreeMap<SharedString, (i64, u32)> = BTreeMap::new();
        let merging = self.merge.iter().flat_map(|m| m.lines.iter());
        for e in self.buffer.entries().chain(merging).chain(self.held.iter()) {
            let (Kind::Line, Some(at)) = (e.kind, e.at) else { continue };
            match out.get_mut(&e.pod) {
                None => {
                    out.insert(e.pod.clone(), (at, 1));
                }
                Some((t, c)) if at < *t => (*t, *c) = (at, 1),
                Some((t, c)) if at == *t => *c += 1,
                Some(_) => {}
            }
        }
        out
    }

    /// Reads back, once, everything the cluster serves of each pod not at its start yet;
    /// [`ServiceLogsView::read_back_done`] gets the lines. False when nothing was started
    /// (a read under way, full, every pod at its start, hidden).
    pub fn earlier(&mut self, cluster: &Entity<Cluster>, cx: &mut Context<ServiceLogsView>) -> bool {
        if self.reading.is_some() || !self.visible || self.last == Some(BackEnd::Full) {
            return false;
        }
        let oldest = self.oldest();
        let generation = self.generation;
        let mut pods = Vec::new();
        for (name, feed) in &mut self.pods {
            if !feed.readable() || feed.start == Some((feed.seen.0, generation)) {
                continue;
            }
            match oldest.get(name) {
                Some(o) => pods.push((name.clone(), feed.seen.0, *o)),
                // Nothing of it shown: nothing to line earlier lines up with.
                None => feed.start = Some((feed.seen.0, generation)),
            }
        }
        if pods.is_empty() {
            return false;
        }
        let (env, service) = (self.env.clone(), self.name.clone());
        let mut streams: Vec<(SharedString, Result<LogStream, String>)> = Vec::new();
        for (name, _, _) in &pods {
            let container = cluster.read(cx).pods(&env, &service).iter().find(|p| p.name == name.as_ref()).and_then(|p| self.container_of(p));
            let stream = match container {
                Some(c) => cluster.update(cx, |cl, cx| cl.read_logs(&env, name, &c, cx)),
                None => Err(format!("pod {} has no such container", short(name))),
            };
            streams.push((name.clone(), stream));
        }
        self.next_id += 1;
        let id = self.next_id;
        let task = cx.spawn(async move |this, cx| {
            let mut results: Vec<ReadResult> = Vec::new();
            for (pod, stream) in streams {
                let result = match stream {
                    Err(why) => Err(why),
                    Ok(LogStream { updates, keep }) => {
                        let _keep = keep;
                        let mut lines = Vec::new();
                        let mut end = Err("the read stopped".to_string());
                        while let Ok(update) = updates.recv().await {
                            match update {
                                LogUpdate::Lines(l) => lines.extend(l),
                                LogUpdate::Problem(_) => {}
                                LogUpdate::Ended => {
                                    end = Ok(());
                                    break;
                                }
                                LogUpdate::Gone => {
                                    end = Err(format!("pod {} is gone", short(&pod)));
                                    break;
                                }
                                LogUpdate::Refused(r) => {
                                    end = Err(refusal_text(&r));
                                    break;
                                }
                            }
                        }
                        end.map(|()| lines)
                    }
                };
                results.push((pod, result));
            }
            let _ = this.update(cx, |v, cx| v.read_back_done(id, results, cx));
        });
        self.last = None;
        self.reading = Some(Reading { id, _task: task, container: self.container.clone(), generation, pods });
        true
    }

    /// A read back ended: each pod's lines older than those shown, merged by time, go
    /// above. `None`: not this read's, or dropped (the container, the generation or a
    /// pod's restarts changed, or the view was hidden meanwhile).
    pub fn finish_read(&mut self, id: u64, results: Vec<ReadResult>) -> Option<Prepended> {
        let reading = self.reading.take_if(|r| r.id == id)?;
        if !self.visible || reading.container != self.container || reading.generation != self.generation {
            return None;
        }
        if reading.pods.iter().any(|(pod, restarts, _)| self.pods.get(pod).is_some_and(|f| f.seen.0 != *restarts)) {
            return None;
        }
        let mut kept = Vec::new();
        let mut unaligned = Vec::new();
        let mut failed = None;
        for (pod, result) in results {
            let Some(&(_, restarts, oldest)) = reading.pods.iter().find(|(p, _, _)| *p == pod) else { continue };
            match result {
                Err(why) => {
                    failed.get_or_insert(why);
                    continue;
                }
                Ok(lines) => match cut(&lines, oldest) {
                    Some(n) => kept.push(lines.into_iter().take(n).map(|l| Entry::line(&pod, l)).collect::<Vec<_>>()),
                    None => unaligned.push(short(&pod).to_string()),
                },
            }
            if let Some(f) = self.pods.get_mut(&pod) {
                f.start = Some((restarts, self.generation));
            }
        }
        let added = self.buffer.prepend(merge_by_time(kept));
        self.last = match failed {
            Some(why) => Some(BackEnd::Failed(why)),
            None if added.full => Some(BackEnd::Full),
            None if !unaligned.is_empty() => Some(BackEnd::Unaligned(unaligned)),
            None => None,
        };
        Some(added)
    }
}

/// How many of a pod's read lines (oldest first) are older than those shown, whose oldest
/// is at `at` with `shown` lines at exactly that time: the lines before the first one at
/// `at`, then those of that time not shown yet (the shown ones are its last; lines without
/// a time stay where they are). `None`: no line at `at`, nothing to line up with.
pub fn cut(lines: &[LogLine], (at, shown): (i64, u32)) -> Option<usize> {
    let first = lines.iter().position(|l| l.at == Some(at))?;
    let group = lines[first..].iter().take_while(|l| l.at.is_none_or(|t| t == at)).filter(|l| l.at == Some(at)).count();
    let mut left = group.saturating_sub(shown as usize);
    let mut keep = first;
    for l in &lines[first..] {
        if left == 0 {
            break;
        }
        match l.at {
            Some(t) if t == at => left -= 1,
            Some(_) => break,
            None => {}
        }
        keep += 1;
    }
    Some(keep)
}

/// Every pod's lines (each oldest first) in one list by time; a line without a time
/// stays after its pod's line before it.
fn merge_by_time(pods: Vec<Vec<Entry>>) -> Vec<Entry> {
    let mut all: Vec<(i64, Entry)> = Vec::with_capacity(pods.iter().map(Vec::len).sum());
    for lines in pods {
        let mut last = i64::MIN;
        for e in lines {
            if let Some(at) = e.at {
                last = at;
            }
            all.push((last, e));
        }
    }
    // Stable: a pod's lines at one time keep their order.
    all.sort_by_key(|(at, _)| *at);
    all.into_iter().map(|(_, e)| e).collect()
}

/// `100000` → `100 000`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// A refusal in words.
pub fn refusal_text(r: &LogRefusal) -> String {
    match r {
        LogRefusal::Forbidden(why) | LogRefusal::Waiting(why) | LogRefusal::NoPrevious(why) | LogRefusal::NoContainer(why) | LogRefusal::Other(why) => why.clone(),
    }
}

/// Reads a stream's updates into the view's feed.
fn read(stream: crate::cluster::LogStream, pod: SharedString, id: u64, cx: &mut Context<ServiceLogsView>) -> Task<()> {
    let crate::cluster::LogStream { updates, keep } = stream;
    cx.spawn(async move |this, cx| {
        let _keep = keep;
        while let Ok(update) = updates.recv().await {
            if this.update(cx, |v, cx| v.feed.receive(pod.clone(), id, update, cx)).is_err() {
                return;
            }
        }
    })
}

/// Whether `text` holds `query` (lowercase characters), ignoring case, per character.
fn holds(text: &str, query: &[char]) -> bool {
    let mut starts = text.char_indices();
    loop {
        let mut rest = starts.as_str().chars();
        if query.iter().all(|q| rest.next().is_some_and(|c| lower(c) == Some(*q))) {
            return true;
        }
        if starts.next().is_none() {
            return false;
        }
    }
}

/// `c` lowercase, when that's one character.
fn lower(c: char) -> Option<char> {
    let mut l = c.to_lowercase();
    let first = l.next();
    if l.next().is_none() { first } else { None }
}

/// Where `query` (lowercase characters) is in `text`, ignoring case, as byte ranges of
/// `text`: every match, left to right, none overlapping.
pub fn match_ranges(text: &str, query: &[char]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    if text.is_ascii() && query.iter().all(char::is_ascii) {
        let t = text.as_bytes();
        let mut i = 0;
        while i + query.len() <= t.len() {
            if t[i..i + query.len()].iter().zip(query).all(|(b, q)| b.to_ascii_lowercase() == *q as u8) {
                out.push(i..i + query.len());
                i += query.len();
            } else {
                i += 1;
            }
        }
        return out;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i + query.len() <= chars.len() {
        if query.iter().enumerate().all(|(k, q)| lower(chars[i + k].1) == Some(*q)) {
            let end = chars.get(i + query.len()).map_or(text.len(), |(b, _)| *b);
            out.push(chars[i].0..end);
            i += query.len();
        } else {
            i += 1;
        }
    }
    out
}
