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
pub mod handlers;
pub mod identity;
pub mod policy;
pub mod socket;
pub mod transport;

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
/// A caller connecting again within this is audited once.
const CONNECT_AUDIT_EVERY: Duration = Duration::from_secs(60);

/// A caller, as devdeck found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    /// A process started from a devdeck shell: that shell's instance.
    Terminal {
        workspace: String,
        n: u32,
        uid: String,
        /// The shell's pid and its run's start count: when either changes, it's gone.
        shell: u32,
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
    pub since: SystemTime,
    pub calls: u64,
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
    /// Where the whole audit goes (none: kept in memory only).
    pub audit_path: Option<PathBuf>,
    writer: Option<audit::Writer>,
    connects: HashMap<String, Instant>,
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
    Open(Peer, SyncSender<Option<Opened>>),
    Call { conn: u64, method: String, params: Value, reply: SyncSender<Result<Value, ApiError>> },
    Notification { conn: u64, method: String },
    Close(u64),
    Refused(String),
}

/// Weak: the channel closes when [`ApiState`] goes (on the UI thread), never from a
/// socket thread.
struct ChannelSink(async_channel::WeakSender<ToUi>);

impl ChannelSink {
    fn send(&self, msg: ToUi) -> Option<()> {
        self.0.upgrade()?.send_blocking(msg).ok()
    }
}

impl Sink for ChannelSink {
    fn open(&self, peer: Peer) -> Option<Opened> {
        let (tx, rx) = sync_channel(1);
        self.send(ToUi::Open(peer, tx))?;
        rx.recv().ok().flatten()
    }

    fn call(&self, conn: u64, method: String, params: Value) -> Result<Value, ApiError> {
        let (reply, rx) = sync_channel(1);
        let gone = || ApiError::unavailable("devdeck is quitting");
        self.send(ToUi::Call { conn, method, params, reply }).ok_or_else(gone)?;
        rx.recv().map_err(|_| gone())?
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
        cx.defer(move |cx| devdeck_ui::overlay::toasts(cx).update(cx, |t, cx| t.push(devdeck_ui::overlay::ToastKind::Warn, text, vec![], cx)));
    }
    let state = cx.new(|_| ApiState {
        status,
        instance: instance(),
        state: kind,
        connections: BTreeMap::new(),
        audit: VecDeque::new(),
        audit_path,
        writer,
        connects: HashMap::new(),
        next: 1,
        _serving: serving,
        _inbox: tx,
    });
    cx.set_global(ApiHandle(state.clone()));
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
        ToUi::Open(peer, reply) => {
            let _ = reply.send(Some(open(&state, &peer, cx)));
        }
        ToUi::Call { conn, method, params, reply } => {
            let _ = reply.send(call(&state, conn, &method, params, cx));
        }
        ToUi::Notification { conn, method } => state.update(cx, |s, cx| {
            let caller = s.connections.get(&conn).map_or_else(|| "?".into(), |c| c.who.label());
            s.record(audit::Entry::new(caller, audit::Event::Notification, Some(method), None), cx);
        }),
        ToUi::Close(conn) => state.update(cx, |s, cx| {
            if s.connections.remove(&conn).is_some() {
                cx.notify();
            }
        }),
        ToUi::Refused(why) => state.update(cx, |s, cx| s.record(audit::Entry::new("?".into(), audit::Event::Refused, None, Some(why)), cx)),
    }
}

/// A new connection from `peer`: who it is, recorded.
pub fn open(state: &Entity<ApiState>, peer: &Peer, cx: &mut App) -> Opened {
    let who = identify(peer, cx);
    state.update(cx, |s, cx| s.open(who, cx))
}

/// A call on a connection: its caller must still be there, then the method runs.
pub fn call(state: &Entity<ApiState>, conn: u64, method: &str, params: Value, cx: &mut App) -> Result<Value, ApiError> {
    let (who, instance, kind) = {
        let s = state.read(cx);
        let c = s.connections.get(&conn).ok_or_else(|| ApiError::new(ErrorKind::Expired, "connection closed"))?;
        (c.who.clone(), s.instance.clone(), s.state)
    };
    if !alive(&who, cx) {
        state.update(cx, |s, cx| s.record(audit::Entry::new(who.label(), audit::Event::Expired, Some(method.into()), None), cx));
        return Err(ApiError::new(ErrorKind::Expired, "the caller's shell ended; connect again"));
    }
    let cx_call = dispatch::CallCx { who: &who, instance: &instance, state: kind };
    let result = dispatch::call(&cx_call, method, params, cx);
    state.update(cx, |s, cx| {
        if let Some(c) = s.connections.get_mut(&conn) {
            c.calls += 1;
        }
        if let Err(e) = &result
            && e.kind == ErrorKind::PermissionDenied
        {
            s.record(audit::Entry::new(who.label(), audit::Event::Denied, Some(method.into()), None), cx);
        }
        cx.notify();
    });
    result
}

/// The caller of a connection: the closest of its ancestors that is a shell devdeck
/// runs, else outside.
pub fn identify(peer: &Peer, cx: &App) -> Who {
    let outside = Who::Outside { pid: peer.pid, program: peer.program.clone() };
    let Some(shell) = crate::shell::try_shell(cx) else { return outside };
    let mut shells: HashMap<u32, Who> = HashMap::new();
    for w in &shell.read(cx).workspaces {
        let runner = w.services.read(cx).runner.read(cx);
        for (key, pid) in runner.pids() {
            let (Some(n), Some(uid), Some(run)) = (key.terminal_number(), runner.terminal_uid(&key), runner.run(&key)) else { continue };
            shells.insert(pid, Who::Terminal { workspace: w.id.clone(), n, uid: uid.to_string(), shell: pid, starts: run.runs });
        }
    }
    peer.chain.iter().find_map(|p| shells.get(p).cloned()).unwrap_or(outside)
}

/// Whether the caller is still there: a terminal's shell instance still runs.
pub fn alive(who: &Who, cx: &App) -> bool {
    let Who::Terminal { workspace, n, shell, starts, .. } = who else { return true };
    let Some(sh) = crate::shell::try_shell(cx) else { return false };
    let Some(w) = sh.read(cx).workspaces.iter().find(|w| &w.id == workspace) else { return false };
    let runner = w.services.read(cx).runner.read(cx);
    let key = crate::runner::RunKey::terminal(*n);
    let running = runner.run(&key).is_some_and(|r| r.runs == *starts && r.state.is_running());
    running && runner.pids().iter().any(|(k, p)| *k == key && p == shell)
}

impl ApiState {
    fn open(&mut self, who: Who, cx: &mut Context<Self>) -> Opened {
        let id = self.next;
        self.next += 1;
        let outside = who.is_outside();
        let label = who.label();
        let now = Instant::now();
        if self.connects.get(&label).is_none_or(|t| now.duration_since(*t) >= CONNECT_AUDIT_EVERY) {
            self.connects.insert(label.clone(), now);
            self.record(audit::Entry::new(label, audit::Event::Connected, None, None), cx);
        }
        self.connections.insert(id, Connection { who, since: SystemTime::now(), calls: 0 });
        cx.notify();
        Opened { id, outside }
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
