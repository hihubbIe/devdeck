//! The control API's server (R13): the one part of devdeck that knows the API, and the
//! one part of the API that knows devdeck. The protocol is `devdeck-api`'s.
//!
//! The socket's threads ([`transport`]) hand each connection and request to the UI
//! thread through a channel; there [`ApiState`] finds who calls ([`identify`]), checks
//! their shell still runs, and dispatches ([`dispatch`]). The answer goes back on a
//! channel of one, and the connection's thread writes it. Nothing runs while nobody
//! calls (R1.3).

pub mod audit;
pub mod dispatch;
pub mod events;
pub mod gitports;
pub mod present;
pub mod handlers;
pub mod identity;
pub mod policy;
pub mod socket;
pub mod transport;
pub mod waits;

use devdeck_api::{ApiError, Caller, ErrorKind};
use gpui::{App, AppContext, Context, Entity, Global};
use identity::Peer;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::{Duration, Instant, SystemTime};
use transport::{Opened, Sink};

/// Audit entries kept for the Settings view.
const AUDIT_KEPT: usize = 200;
/// A caller's actions are told in a toast at most this often (the audit has them all).
const NOTICE_EVERY: Duration = Duration::from_secs(2);
/// The arguments an audit entry keeps, in characters.
const AUDIT_ARGS: usize = 200;
/// A caller connecting again within this is audited once.
const CONNECT_AUDIT_EVERY: Duration = Duration::from_secs(60);
/// Connections one caller may hold at once (one can't lock the others out).
pub const CONNECTIONS_PER_CALLER: usize = 8;

/// A caller, as devdeck found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    /// A process started from a devdeck shell: that shell's instance.
    Terminal {
        workspace: String,
        n: u32,
        uid: String,
        /// The shell's pid, its start time and its run's start count: when any changes,
        /// it's gone.
        shell: u32,
        shell_start: u64,
        starts: u32,
    },
    Outside { pid: Option<u32>, program: String },
}

impl Who {
    pub fn caller(&self) -> Caller {
        match self {
            Who::Terminal { workspace, n, uid, .. } => Caller::Terminal { workspace: workspace.clone(), n: *n, uid: uid.clone() },
            Who::Outside { pid, program } => Caller::Outside { pid: *pid, program: program.as_str().into() },
        }
    }

    /// As people read it: `work · terminal 3`, `devdeck-ctl (pid 4242)`.
    pub fn label(&self) -> String {
        match self {
            Who::Terminal { workspace, n, .. } => format!("{workspace} · terminal {n}"),
            Who::Outside { pid: Some(p), program } => format!("{program} (pid {p})"),
            Who::Outside { pid: None, program } => format!("{program} (unpinned)"),
        }
    }

    pub fn is_outside(&self) -> bool {
        matches!(self, Who::Outside { .. })
    }
}

/// Where the API's settings come from (`main`; tests give their own).
pub struct Config {
    /// `$XDG_RUNTIME_DIR`; None: the API is off.
    pub runtime: Option<PathBuf>,
    pub state_dir: PathBuf,
    /// The state folder devdeck uses when nothing overrides it (`main` is that one).
    pub default_state_dir: PathBuf,
    /// Write the audit file (the state folder's).
    pub audit_file: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    On(PathBuf),
    /// Why it's off.
    Off(String),
}

pub struct Connection {
    pub who: Who,
    /// The connected process, when it was pinned.
    pub pid: Option<u32>,
    pub since: SystemTime,
    pub calls: u64,
    /// Where its events go, and the topics it follows.
    events: Option<events::Outbox>,
    pub topics: std::collections::BTreeSet<devdeck_api::types::Topic>,
}

pub struct ApiState {
    pub status: Status,
    /// This run of devdeck: pid and start time.
    pub instance: String,
    /// `main`, `demo` or `other`.
    pub state: &'static str,
    pub connections: BTreeMap<u64, Connection>,
    /// Newest first.
    pub audit: VecDeque<audit::Entry>,
    /// Answers coming later, by connection (one call in flight on each): dropped, the
    /// call stops (its client hung up, devdeck quits).
    later: HashMap<u64, gpui::Task<()>>,
    /// Watching the models while some connection follows a topic; what changed since
    /// the last send, and the send waiting.
    hub: Option<events::Hub>,
    pending: BTreeMap<(devdeck_api::types::Topic, String), events::Change>,
    flushing: Option<gpui::Task<()>>,
    /// Whether a caller is still there ([`alive`]; tests give their own: the demo's
    /// shells have no process).
    pub alive: fn(&Who, &App) -> bool,
    /// Where the whole audit goes (none: kept in memory only).
    pub audit_path: Option<PathBuf>,
    writer: Option<audit::Writer>,
    connects: HashMap<String, Instant>,
    /// When each caller was last told in a toast.
    noticed: HashMap<String, Instant>,
    next: u64,
    _serving: Option<(socket::Bound, transport::Stop)>,
    /// The socket threads' channel to the UI thread (they hold it weakly).
    _inbox: async_channel::Sender<ToUi>,
}

struct ApiHandle(Entity<ApiState>);
impl Global for ApiHandle {}

pub fn state(cx: &App) -> Option<Entity<ApiState>> {
    cx.try_global::<ApiHandle>().map(|h| h.0.clone())
}

/// The socket shells are told about, while the API is on.
pub fn socket_path(cx: &App) -> Option<PathBuf> {
    match &state(cx)?.read(cx).status {
        Status::On(p) => Some(p.clone()),
        Status::Off(_) => None,
    }
}

/// What the socket's threads hand to the UI thread.
enum ToUi {
    Open(Peer, SyncSender<Vec<u8>>, SyncSender<Option<Opened>>),
    Call { conn: u64, method: String, params: Value, reply: SyncSender<Result<Value, ApiError>> },
    Notification { conn: u64, method: String },
    Close(u64),
    /// The client hung up while its call was answered later.
    Cancel(u64),
    Refused(String),
}

/// While a call waits for its answer, how often its client is checked for having hung up.
const HANG_UP_CHECK: Duration = Duration::from_millis(250);

/// Weak: the channel closes when [`ApiState`] goes (on the UI thread), never from a
/// socket thread.
struct ChannelSink(async_channel::WeakSender<ToUi>);

impl ChannelSink {
    fn send(&self, msg: ToUi) -> Option<()> {
        self.0.upgrade()?.send_blocking(msg).ok()
    }
}

impl Sink for ChannelSink {
    fn open(&self, peer: Peer, events: SyncSender<Vec<u8>>) -> Option<Opened> {
        let (tx, rx) = sync_channel(1);
        self.send(ToUi::Open(peer, events, tx))?;
        rx.recv().ok().flatten()
    }

    fn call(&self, conn: u64, method: String, params: Value, hung_up: &dyn Fn() -> bool) -> Result<Value, ApiError> {
        let (reply, rx) = sync_channel(1);
        let gone = || ApiError::unavailable("devdeck is quitting");
        self.send(ToUi::Call { conn, method, params, reply }).ok_or_else(gone)?;
        loop {
            match rx.recv_timeout(HANG_UP_CHECK) {
                Ok(r) => return r,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(gone()),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) if hung_up() => {
                    self.send(ToUi::Cancel(conn));
                    return Err(ApiError::new(ErrorKind::Expired, "the client hung up"));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    fn notification(&self, conn: u64, method: String) {
        self.send(ToUi::Notification { conn, method });
    }

    fn close(&self, conn: u64) {
        self.send(ToUi::Close(conn));
    }

    fn refused(&self, why: String) {
        self.send(ToUi::Refused(why));
    }
}

/// This process: `pid-start`, as `core::process` names instances.
fn instance() -> String {
    let pid = std::process::id();
    format!("{pid}-{}", devdeck_core::procfs::parent_and_start(pid).map_or(0, |(_, s)| s))
}

/// Starts the API: binds the socket and serves it, or says why it's off (a toast and
/// the Settings view).
pub fn start(config: Config, cx: &mut App) -> Entity<ApiState> {
    let (name, kind) = socket::name(&config.state_dir, &config.default_state_dir);
    let bound = match &config.runtime {
        None => Err("XDG_RUNTIME_DIR isn't set".to_string()),
        Some(rt) => socket::bind(rt, &name),
    };
    let audit_path = config.audit_file.then(|| config.state_dir.join(audit::FILE));
    let writer = audit_path.clone().and_then(|p| audit::Writer::start(p).ok());
    let (tx, rx) = async_channel::unbounded::<ToUi>();
    let (status, serving) = match bound.and_then(|b| {
        let listener = b.listener.try_clone().map_err(|e| e.to_string())?;
        let stop = transport::serve(listener, Arc::new(ChannelSink(tx.downgrade()))).map_err(|e| e.to_string())?;
        Ok((b, stop))
    }) {
        Ok((b, stop)) => (Status::On(b.path.clone()), Some((b, stop))),
        Err(why) => (Status::Off(why), None),
    };
    if let Status::Off(why) = &status {
        let text = format!("The control API is off: {why}");
        cx.defer(move |cx| {
            devdeck_ui::overlay::toasts(cx).update(cx, |t, cx| t.push(devdeck_ui::overlay::ToastKind::Warn, text, vec![], cx));
        });
    }
    let state = cx.new(|_| ApiState {
        status,
        instance: instance(),
        state: kind,
        connections: BTreeMap::new(),
        audit: VecDeque::new(),
        later: HashMap::new(),
        hub: None,
        pending: BTreeMap::new(),
        flushing: None,
        alive,
        audit_path,
        writer,
        connects: HashMap::new(),
        noticed: HashMap::new(),
        next: 1,
        _serving: serving,
        _inbox: tx,
    });
    cx.set_global(ApiHandle(state.clone()));
    let quitting = state.downgrade();
    cx.on_app_quit(move |cx| {
        if let Some(s) = quitting.upgrade() {
            s.update(cx, |s, _| s.close());
        }
        async {}
    })
    .detach();
    let weak = state.downgrade();
    cx.spawn(async move |cx| {
        while let Ok(msg) = rx.recv().await {
            cx.update(|cx| on_message(&weak, msg, cx));
        }
    })
    .detach();
    state
}

fn on_message(state: &gpui::WeakEntity<ApiState>, msg: ToUi, cx: &mut App) {
    let Some(state) = state.upgrade() else { return };
    match msg {
        ToUi::Open(peer, events, reply) => {
            let _ = reply.send(open(&state, &peer, Some(events), cx));
        }
        ToUi::Call { conn, method, params, reply } => match answer(&state, conn, &method, params, cx) {
            dispatch::Answer::Now(r) => {
                let _ = reply.send(r);
            }
            dispatch::Answer::Later(task) => {
                let t = cx.spawn(async move |_| {
                    let _ = reply.send(task.await);
                });
                state.update(cx, |s, _| s.later.insert(conn, t));
            }
        },
        ToUi::Cancel(conn) => state.update(cx, |s, _| {
            s.later.remove(&conn);
        }),
        ToUi::Notification { conn, method } => state.update(cx, |s, cx| {
            let caller = s.connections.get(&conn).map_or_else(|| "?".into(), |c| c.who.label());
            s.record(audit::Entry::new(caller, audit::Event::Notification, Some(method), None), cx);
        }),
        ToUi::Close(conn) => state.update(cx, |s, cx| s.close_connection(conn, cx)),
        ToUi::Refused(why) => state.update(cx, |s, cx| {
            // Once a minute per reason: a client connecting in a loop doesn't fill the audit.
            if s.first_in_a_while(format!("refused: {why}")) {
                s.record(audit::Entry::new("?".into(), audit::Event::Refused, None, Some(why)), cx);
            }
        }),
    }
}

/// A new connection from `peer`: who it is, recorded; None past
/// [`CONNECTIONS_PER_CALLER`].
/// `events`: where its events go (none: it gets none).
pub fn open(state: &Entity<ApiState>, peer: &Peer, events: Option<SyncSender<Vec<u8>>>, cx: &mut App) -> Option<Opened> {
    let who = identify(peer, cx);
    state.update(cx, |s, cx| s.open(who, peer.pid, events, cx))
}

/// Tells the user what a caller did, in a toast: one per caller every
/// [`NOTICE_EVERY`] at most (an agent in a loop doesn't flood them).
pub fn notice(who: &Who, text: String, cx: &mut App) {
    let Some(state) = state(cx) else { return };
    let now = Instant::now();
    let tell = state.update(cx, |s, _| {
        let label = who.label();
        s.noticed.retain(|_, t| now.duration_since(*t) < NOTICE_EVERY);
        s.noticed.insert(label, now).is_none()
    });
    if tell {
        devdeck_ui::overlay::toasts(cx).update(cx, |t, cx| t.push(devdeck_ui::overlay::ToastKind::Info, text, vec![], cx));
    }
}

/// [`answer`] for a method that answers now (tests).
pub fn call(state: &Entity<ApiState>, conn: u64, method: &str, params: Value, cx: &mut App) -> Result<Value, ApiError> {
    match answer(state, conn, method, params, cx) {
        dispatch::Answer::Now(r) => r,
        dispatch::Answer::Later(_) => Err(ApiError::invalid(format!("{method} answers later"))),
    }
}

/// A call on a connection: its caller must still be there, then the method runs. Its
/// outcome is audited when it's known (a wait's, once it ends).
pub fn answer(state: &Entity<ApiState>, conn: u64, method: &str, params: Value, cx: &mut App) -> dispatch::Answer {
    let (who, pid, instance, kind) = {
        let s = state.read(cx);
        let Some(c) = s.connections.get(&conn) else { return dispatch::Answer::Now(Err(ApiError::new(ErrorKind::Expired, "connection closed"))) };
        (c.who.clone(), c.pid, s.instance.clone(), s.state)
    };
    if !alive(&who, cx) {
        state.update(cx, |s, cx| s.record(audit::Entry::new(who.label(), audit::Event::Expired, Some(method.into()), None), cx));
        return dispatch::Answer::Now(Err(ApiError::new(ErrorKind::Expired, "the caller's shell ended; connect again")));
    }
    let cx_call = dispatch::CallCx { who: &who, instance: &instance, state: kind, conn: Some(conn) };
    let args: String = params.to_string().chars().take(AUDIT_ARGS).collect();
    match dispatch::answer(&cx_call, method, params, cx) {
        dispatch::Answer::Now(result) => {
            audited(state, conn, &who, pid, method, &args, &result, cx);
            dispatch::Answer::Now(result)
        }
        dispatch::Answer::Later(task) => {
            let (state, method) = (state.clone(), method.to_string());
            dispatch::Answer::Later(cx.spawn(async move |cx| {
                // The caller's shell ending ends the call (looked at as often as a wait).
                let gone = async {
                    loop {
                        cx.background_executor().timer(waits::TICK).await;
                        if !cx.update(|cx| alive(&who, cx)) {
                            return Err(ApiError::new(ErrorKind::Expired, "the caller's shell ended"));
                        }
                    }
                };
                let result = futures_lite::future::or(task, gone).await;
                cx.update(|cx| audited(&state, conn, &who, pid, &method, &args, &result, cx));
                result
            }))
        }
    }
}

/// Records a call's outcome: one entry per call that acts, a look only when refused.
#[allow(clippy::too_many_arguments)]
fn audited(state: &Entity<ApiState>, conn: u64, who: &Who, pid: Option<u32>, method: &str, args: &str, result: &Result<Value, ApiError>, cx: &mut App) {
    let acts = dispatch::info(method).is_some_and(|m| m.class != devdeck_api::Class::Observe);
    state.update(cx, |s, cx| {
        if let Some(c) = s.connections.get_mut(&conn) {
            c.calls += 1;
        }
        let outcome = match result {
            Ok(_) => "done".to_string(),
            Err(e) => e.kind.as_str().to_string(),
        };
        if acts {
            s.record(audit::Entry::new(who.label(), audit::Event::Called, Some(method.into()), Some(format!("{args} → {outcome}"))).with_pid(pid), cx);
        } else if let Err(e) = result
            && e.kind == ErrorKind::PermissionDenied
        {
            s.record(audit::Entry::new(who.label(), audit::Event::Denied, Some(method.into()), None).with_pid(pid), cx);
        }
        cx.notify();
    });
}

/// The caller of a connection: the closest of its ancestors that is a shell devdeck
/// runs, else outside.
pub fn identify(peer: &Peer, cx: &App) -> Who {
    let outside = Who::Outside { pid: peer.pid, program: peer.program.clone() };
    let Some(shell) = crate::shell::try_shell(cx) else { return outside };
    let mut shells: HashMap<(u32, u64), Who> = HashMap::new();
    for w in &shell.read(cx).workspaces {
        let runner = w.services.read(cx).runner.read(cx);
        for (key, pid, start) in runner.processes() {
            let (Some(n), Some(uid), Some(run)) = (key.terminal_number(), runner.terminal_uid(&key), runner.run(&key)) else { continue };
            shells.insert((pid, start), Who::Terminal { workspace: w.id.clone(), n, uid: uid.to_string(), shell: pid, shell_start: start, starts: run.runs });
        }
    }
    // Pid and start time: a pid the kernel gave again before the runner heard its shell
    // end isn't that shell.
    peer.chain.iter().find_map(|p| shells.get(p).cloned()).unwrap_or(outside)
}

/// Whether the caller is still there: a terminal's shell instance still runs.
pub fn alive(who: &Who, cx: &App) -> bool {
    let Who::Terminal { workspace, n, shell, shell_start, starts, .. } = who else { return true };
    let Some(sh) = crate::shell::try_shell(cx) else { return false };
    let Some(w) = sh.read(cx).workspaces.iter().find(|w| &w.id == workspace) else { return false };
    let runner = w.services.read(cx).runner.read(cx);
    let key = crate::runner::RunKey::terminal(*n);
    let running = runner.run(&key).is_some_and(|r| r.runs == *starts && r.state.is_running());
    running && runner.processes().iter().any(|(k, p, s)| *k == key && p == shell && s == shell_start)
}

impl ApiState {
    /// A connection of `who` (tests: a caller of their own).
    pub fn open(&mut self, who: Who, pid: Option<u32>, events: Option<SyncSender<Vec<u8>>>, cx: &mut Context<Self>) -> Option<Opened> {
        if self.connections.values().filter(|c| c.who == who).count() >= CONNECTIONS_PER_CALLER {
            if self.first_in_a_while(format!("too many: {}", who.label())) {
                self.record(audit::Entry::new(who.label(), audit::Event::Refused, None, Some(format!("more than {CONNECTIONS_PER_CALLER} connections"))), cx);
            }
            return None;
        }
        let id = self.next;
        self.next += 1;
        let outside = who.is_outside();
        let label = who.label();
        if self.first_in_a_while(label.clone()) {
            self.record(audit::Entry::new(label, audit::Event::Connected, None, None).with_pid(pid), cx);
        }
        self.connections.insert(id, Connection { who, pid, since: SystemTime::now(), calls: 0, events: events.map(events::Outbox::new), topics: Default::default() });
        cx.notify();
        Some(Opened { id, outside })
    }

    /// A connection ended: its call answered later stops, its events go (its writer ends),
    /// and the models stop being watched if it was the last to follow them.
    pub fn close_connection(&mut self, conn: u64, cx: &mut Context<Self>) {
        self.later.remove(&conn);
        if self.connections.remove(&conn).is_some() {
            self.watch(cx);
            cx.notify();
        }
    }

    /// Stops serving and removes the socket (devdeck quitting).
    fn close(&mut self) {
        // Waits answer `unavailable` (their reply goes with them) before the socket goes.
        self.later.clear();
        if let Some((bound, mut stop)) = self._serving.take() {
            stop.stop();
            bound.remove();
        }
    }

    /// Whether `key` wasn't seen within [`CONNECT_AUDIT_EVERY`]; forgets keys older than
    /// that (callers' labels hold pids, so they'd pile up).
    fn first_in_a_while(&mut self, key: String) -> bool {
        let now = Instant::now();
        self.connects.retain(|_, t| now.duration_since(*t) < CONNECT_AUDIT_EVERY);
        if self.connects.contains_key(&key) {
            return false;
        }
        self.connects.insert(key, now);
        true
    }

    fn record(&mut self, e: audit::Entry, cx: &mut Context<Self>) {
        if let Some(w) = &self.writer {
            w.write(e.clone());
        }
        self.audit.push_front(e);
        self.audit.truncate(AUDIT_KEPT);
        cx.notify();
    }
}
