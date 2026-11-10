//! What a logs view shows (R7.5): [`LogsBuffer`], the lines kept with what the filter and
//! find show, kept up to date as lines come and go (nothing is recomputed per frame); and
//! [`LogsFeed`], one stream per pod of the service while the view is on screen.
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
//! - Paused, lines go to a queue of their own (capped and counted) shown on resume.

use super::logs_view::{MinLevel, ServiceLogsView};
use crate::cluster::{Cluster, LogFrom, LogLine, LogRefusal, LogUpdate};
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
pub const MAX_LINES: usize = 10_000;
/// Bytes of text kept for the whole view.
pub const MAX_BYTES: usize = 16 << 20;
/// How long a gone pod's stream may run to its end.
pub const GONE_GRACE: Duration = Duration::from_secs(30);
/// How long the first lines of streams started together wait for each other.
pub const MERGE_WAIT: Duration = Duration::from_secs(2);
/// How often updates are drained into the view (about a frame).
pub const DRAIN_EVERY: Duration = Duration::from_millis(16);
/// How long after a stream ended (or was refused as not started) while its container
/// still runs it's asked again: a server or proxy can end a stream cleanly.
pub const ENDED_RETRY: Duration = Duration::from_secs(5);

/// One line shown: a pod's, or a note of devdeck's ("pod … gone").
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The server's timestamp (epoch ns).
    pub at: Option<i64>,
    /// The pod's full name.
    pub pod: SharedString,
    pub text: SharedString,
    pub level: Level,
    pub note: bool,
}

impl Entry {
    fn line(pod: &SharedString, l: LogLine) -> Self {
        Self { at: l.at, pod: pod.clone(), text: l.text.into(), level: l.level, note: false }
    }

    fn note(pod: &SharedString, text: String) -> Self {
        Self { at: None, pod: pod.clone(), text: text.into(), level: Level::Plain, note: true }
    }
}

/// A pod's name as rows show it: its last part (`graphservice-5d9f-x2k8q` → `x2k8q`).
pub fn short(pod: &str) -> &str {
    pod.rsplit('-').next().unwrap_or(pod)
}

/// The lines kept, by absolute index (the first line ever is 0), and what the filter,
/// the pod picked and find show, kept up to date line by line.
pub struct LogsBuffer {
    entries: VecDeque<Entry>,
    /// Each entry's length in characters (finding the longest again costs no counting).
    chars: VecDeque<usize>,
    /// The absolute index of `entries[0]`.
    first: u64,
    /// Lines cleared on purpose (another container), not dropped for room.
    cleared: u64,
    bytes: usize,
    min: MinLevel,
    pod: Option<SharedString>,
    /// The find text's characters, lowercase; empty: no find.
    query: Vec<char>,
    /// Absolute indices of the lines shown, in order.
    rows: VecDeque<u64>,
    /// Absolute indices of the shown lines holding the find text, and how many times each.
    hits: VecDeque<u64>,
    hit_counts: VecDeque<u32>,
    /// Every match of the shown lines.
    total: usize,
    /// The longest line shown, by characters.
    longest: Option<(u64, usize)>,
    max_lines: usize,
    max_bytes: usize,
    /// Moves whenever what's shown changes (wrapping is worked out again then).
    version: u64,
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
            first: 0,
            cleared: 0,
            bytes: 0,
            min: MinLevel::All,
            pod: None,
            query: Vec::new(),
            rows: VecDeque::new(),
            hits: VecDeque::new(),
            hit_counts: VecDeque::new(),
            total: 0,
            longest: None,
            max_lines: max_lines.max(1),
            max_bytes,
            version: 0,
        }
    }

    fn shows(&self, e: &Entry) -> bool {
        // A note about no pod in particular shows whatever pod is picked.
        let pod = e.pod.is_empty() || self.pod.as_ref().is_none_or(|p| *p == e.pod);
        pod && (e.note || super::logs_view::shown(e.level, self.min))
    }

    /// How many times `e` holds the find text.
    fn hit(&self, e: &Entry) -> usize {
        if self.query.is_empty() { 0 } else { match_ranges(&e.text, &self.query).len() }
    }

    pub fn push(&mut self, e: Entry) {
        self.version += 1;
        let abs = self.first + self.entries.len() as u64;
        self.bytes += e.text.len();
        let chars = e.text.chars().count();
        if self.shows(&e) {
            self.rows.push_back(abs);
            let n = self.hit(&e);
            if n > 0 {
                self.hits.push_back(abs);
                self.hit_counts.push_back(n as u32);
                self.total += n;
            }
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
        }
        while self.rows.front().is_some_and(|i| *i < self.first) {
            self.rows.pop_front();
        }
        while self.hits.front().is_some_and(|i| *i < self.first) {
            self.hits.pop_front();
            self.total -= self.hit_counts.pop_front().unwrap_or(0) as usize;
        }
        if lost_longest {
            self.find_longest();
        }
    }

    fn find_longest(&mut self) {
        self.longest = self.rows.iter().map(|i| (*i, self.chars[(*i - self.first) as usize])).max_by_key(|(_, n)| *n);
    }

    fn refilter(&mut self) {
        self.version += 1;
        self.rows = (0..self.entries.len()).filter(|i| self.shows(&self.entries[*i])).map(|i| self.first + i as u64).collect();
        self.find_longest();
        self.refind();
    }

    fn refind(&mut self) {
        self.hits.clear();
        self.hit_counts.clear();
        self.total = 0;
        for i in self.rows.clone() {
            let n = self.hit(&self.entries[(i - self.first) as usize]);
            if n > 0 {
                self.hits.push_back(i);
                self.hit_counts.push_back(n as u32);
                self.total += n;
            }
        }
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

    pub fn set_query(&mut self, query: &str) {
        self.query = query.chars().flat_map(char::to_lowercase).collect();
        self.refind();
    }

    pub fn clear(&mut self) {
        self.version += 1;
        self.first += self.entries.len() as u64;
        self.cleared += self.entries.len() as u64;
        self.entries.clear();
        self.chars.clear();
        self.bytes = 0;
        self.rows.clear();
        self.hits.clear();
        self.hit_counts.clear();
        self.total = 0;
        self.longest = None;
    }

    /// What's shown changed when this did.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Lines kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Lines dropped to stay within the caps.
    pub fn dropped(&self) -> u64 {
        self.first - self.cleared
    }

    /// Lines shown.
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// The line shown at `row`.
    pub fn row(&self, row: usize) -> Option<&Entry> {
        let i = *self.rows.get(row)?;
        self.entries.get((i - self.first) as usize)
    }

    /// The absolute index of the line shown at `row`.
    pub fn row_line(&self, row: usize) -> Option<u64> {
        self.rows.get(row).copied()
    }

    /// The row showing the line `abs`, if it's shown.
    pub fn row_of(&self, abs: u64) -> Option<usize> {
        self.rows.binary_search(&abs).ok()
    }

    /// The row of the longest line shown (sizes the sideways scroll).
    pub fn longest_row(&self) -> Option<usize> {
        self.row_of(self.longest?.0)
    }

    /// Absolute indices of the shown lines holding the find text, in order.
    pub fn hits(&self) -> &VecDeque<u64> {
        &self.hits
    }

    /// Every match of the find text in the shown lines.
    pub fn matches(&self) -> usize {
        self.total
    }

    /// The find text's characters, lowercase.
    pub fn query(&self) -> &[char] {
        &self.query
    }

    /// The `index`th match (from 0): its line and which match of that line.
    pub fn match_at(&self, index: usize) -> Option<(u64, usize)> {
        let mut left = index;
        for (line, n) in self.hits.iter().zip(&self.hit_counts) {
            let n = *n as usize;
            if left < n {
                return Some((*line, left));
            }
            left -= n;
        }
        None
    }

    /// Where the `nth` match of `line` is among every match.
    pub fn match_index(&self, line: u64, nth: usize) -> Option<usize> {
        let at = self.hits.binary_search(&line).ok()?;
        let before: usize = self.hit_counts.iter().take(at).map(|n| *n as usize).sum();
        (nth < self.hit_counts[at] as usize).then_some(before + nth)
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
}

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
    /// Lines that came while paused.
    held: VecDeque<Entry>,
    /// Lines that came while paused and didn't fit.
    held_dropped: u64,
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
            held: VecDeque::new(),
            held_dropped: 0,
        }
    }

    /// Streams running (tests, the leak run).
    pub fn streams(&self) -> usize {
        self.pods.values().filter(|p| p.stream.is_some()).count()
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Lines that came while paused, not shown yet.
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
                    lines.extend(l.into_iter().map(|l| Entry::line(&pod, l)));
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
        if self.paused {
            self.held.push_back(e);
            if self.held.len() > MAX_LINES {
                self.held.pop_front();
                self.held_dropped += 1;
            }
        } else {
            self.buffer.push(e);
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if !paused {
            for e in std::mem::take(&mut self.held) {
                self.buffer.push(e);
            }
            self.held_dropped = 0;
        }
    }
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

/// Where `query` (lowercase characters) is in `text`, ignoring case, as byte ranges of
/// `text`: every match, left to right, none overlapping.
pub fn match_ranges(text: &str, query: &[char]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i + query.len() <= chars.len() {
        let hit = query.iter().enumerate().all(|(k, q)| {
            let mut lower = chars[i + k].1.to_lowercase();
            lower.next() == Some(*q) && lower.next().is_none()
        });
        if hit {
            let end = chars.get(i + query.len()).map_or(text.len(), |(b, _)| *b);
            out.push(chars[i].0..end);
            i += query.len();
        } else {
            i += 1;
        }
    }
    out
}
