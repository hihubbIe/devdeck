//! Live watches of an env's deployments, on one tokio runtime for the whole process,
//! started by the first watch (a workspace without a cluster doesn't pay for it).
//!
//! - One client per kubeconfig file, cluster and user, shared by the envs of the file,
//!   built again when the file's content changes and dropped with its last watch.
//! - A watch follows kube-runtime's watcher on the env's namespace: a list (`Init` …
//!   `InitDone`) replaces what was known, so deployments deleted while the watch was down
//!   disappear; then each change updates it. Deployments are sent [`SETTLE`] after the
//!   first change not sent yet, and only when they differ from what was sent last (a
//!   status that moves without changing the model sends nothing).
//! - The server ends each watch after [`WATCH_TIMEOUT`] and the watcher starts another:
//!   a connection that died without a word (a VPN down, a laptop asleep) shows as a
//!   problem within about a minute, not after kube's default of five.
//! - An error drops the watcher and starts a new one after a backoff (1 s doubling to
//!   30 s, reset by a successful list); the new one lists again, which clears the problem.
//!   kube-runtime 5.0.0's watcher would resume on its own, but silently: after a failed
//!   watch it retries from its last resource version (states `InitListed`, `Watching`)
//!   and, once that works, goes on with no event at all, so nothing would say the problem
//!   is over. It also retries on the very next poll, with no backoff of its own.

use crate::Target;
use crate::convert;
use crate::errors::{describe, describe_watcher};
use devdeck_core::cluster::Deployment;
use futures_lite::StreamExt;
use k8s_openapi::api::apps::v1 as apps;
use kube::config::KubeConfigOptions;
use kube::runtime::watcher::{self, Event, watcher};
use kube::{Api, Client, ResourceExt};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;

/// How long after a first change the deployments are sent: a rollout touching many fields
/// is one update, and steady churn still sends at this pace.
const SETTLE: Duration = Duration::from_millis(100);
/// How long the server keeps one watch open, in seconds.
const WATCH_TIMEOUT: u32 = 60;
/// How long connecting to a cluster may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The first wait before trying again after an error; it doubles up to [`LAST_RETRY`].
const FIRST_RETRY: Duration = Duration::from_secs(1);
const LAST_RETRY: Duration = Duration::from_secs(30);

/// What a watch sends.
#[derive(Clone, Debug, PartialEq)]
pub enum EnvUpdate {
    /// Every deployment of the env, alphabetical, sent when it changed.
    Deployments(Vec<Deployment>),
    /// What's wrong now (`None`: nothing any more).
    Problem(Option<String>),
}

/// Dropping it stops the watch.
pub struct WatchHandle {
    task: tokio::task::AbortHandle,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Starts watching `target`'s deployments. The watch also ends when the receiver is
/// dropped.
pub fn watch(target: &Target) -> (async_channel::Receiver<EnvUpdate>, WatchHandle) {
    let (tx, rx) = async_channel::unbounded();
    let target = target.clone();
    let mut out = Out::new(tx.clone());
    let task = runtime().spawn(until_closed(tx, async move {
        let (client, _user) = connect(&target, &mut out).await;
        feed(client, &target, &mut out).await;
    }));
    (rx, WatchHandle { task: task.abort_handle() })
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("devdeck-kube")
            .enable_all()
            .build()
            .expect("the kube runtime starts")
    })
}

/// Runs `work` until the receiver of `tx` is gone.
async fn until_closed(tx: async_channel::Sender<EnvUpdate>, work: impl Future<Output = ()>) {
    futures_lite::future::or(work, async move { tx.closed().await }).await
}

/// The client for `target`, retried with the backoff while it can't be built.
async fn connect(target: &Target, out: &mut Out) -> (Client, ClientUser) {
    let mut backoff = Backoff::new();
    loop {
        match client(target).await {
            Ok(client) => return client,
            Err(e) => {
                out.problem(Some(describe(&e, target)));
                backoff.wait().await;
            }
        }
    }
}

/// A client's file, cluster and user.
type ClientKey = (PathBuf, String, String);

/// A built client: the fingerprint of the file it was built from, and how many watches
/// use it.
struct Shared {
    fingerprint: u64,
    client: Client,
    users: usize,
}

/// The clients in use.
static CLIENTS: LazyLock<Mutex<HashMap<ClientKey, Shared>>> = LazyLock::new(Default::default);

/// A watch's use of a client: the last one dropped drops the client.
struct ClientUser(ClientKey);

impl Drop for ClientUser {
    fn drop(&mut self) {
        let mut clients = CLIENTS.lock().unwrap();
        if let Some(shared) = clients.get_mut(&self.0) {
            shared.users -= 1;
            if shared.users == 0 {
                clients.remove(&self.0);
            }
        }
    }
}

/// The client of `target`'s file, cluster and user, built on first use (on the runtime:
/// it spawns its buffer's task) and again when the file's content changed.
async fn client(target: &Target) -> Result<(Client, ClientUser), kube::Error> {
    let key = (target.file.clone(), target.cluster.clone(), target.user());
    {
        let mut clients = CLIENTS.lock().unwrap();
        if let Some(shared) = clients.get_mut(&key)
            && shared.fingerprint == target.fingerprint
        {
            shared.users += 1;
            return Ok((shared.client.clone(), ClientUser(key)));
        }
    }
    let options = KubeConfigOptions { context: Some(target.context.clone()), ..Default::default() };
    let mut config = kube::Config::from_custom_kubeconfig((*target.config.0).clone(), &options).await?;
    config.connect_timeout = Some(CONNECT_TIMEOUT);
    let client = Client::try_from(config)?;
    let mut clients = CLIENTS.lock().unwrap();
    // Watches of the old content keep their own clone until they restart.
    let users = clients.get(&key).filter(|s| s.fingerprint == target.fingerprint).map_or(0, |s| s.users);
    clients.insert(key.clone(), Shared { fingerprint: target.fingerprint, client: client.clone(), users: users + 1 });
    Ok((client, ClientUser(key)))
}

/// Sends `target`'s deployments, read through `client`, until dropped.
async fn feed(client: Client, target: &Target, out: &mut Out) {
    let api: Api<apps::Deployment> = Api::namespaced(client, &target.namespace);
    let mut live: BTreeMap<String, Deployment> = BTreeMap::new();
    let mut backoff = Backoff::new();
    loop {
        let failure = {
            let mut stream = std::pin::pin!(watcher(api.clone(), watcher::Config::default().timeout(WATCH_TIMEOUT)));
            let mut listing: Option<BTreeMap<String, Deployment>> = None;
            // When the first change not sent yet is due.
            let mut due: Option<tokio::time::Instant> = None;
            let mut changed = false;
            let failure = loop {
                if changed && due.is_none() {
                    due = Some(tokio::time::Instant::now() + SETTLE);
                }
                let next = match due {
                    Some(at) => match tokio::time::timeout_at(at, stream.next()).await {
                        Ok(next) => next,
                        Err(_) => {
                            (changed, due) = (false, None);
                            out.deployments(&live);
                            continue;
                        }
                    },
                    None => stream.next().await,
                };
                match next {
                    Some(Ok(Event::Init)) => listing = Some(BTreeMap::new()),
                    Some(Ok(Event::InitApply(d))) => apply(listing.get_or_insert_default(), &d),
                    Some(Ok(Event::InitDone)) => {
                        live = listing.take().unwrap_or_default();
                        changed = true;
                        backoff = Backoff::new();
                        out.problem(None);
                    }
                    Some(Ok(Event::Apply(d))) => {
                        apply(&mut live, &d);
                        changed = true;
                    }
                    Some(Ok(Event::Delete(d))) => {
                        live.remove(&d.name_any());
                        changed = true;
                    }
                    Some(Err(e)) => break Some(e),
                    None => break None,
                }
            };
            if changed {
                out.deployments(&live);
            }
            failure
        };
        match failure {
            // An expired resource version: the next list is the fix, not a problem.
            Some(watcher::Error::WatchError(status)) if status.code == 410 => {}
            Some(e) => out.problem(Some(describe_watcher(&e, target))),
            None => {}
        }
        backoff.wait().await;
    }
}

fn apply(map: &mut BTreeMap<String, Deployment>, d: &apps::Deployment) {
    match convert::deployment(d) {
        Some(model) => {
            map.insert(model.name.clone(), model);
        }
        None => {
            map.remove(&d.name_any());
        }
    }
}

/// The sending end of a watch, which sends only what changed.
struct Out {
    tx: async_channel::Sender<EnvUpdate>,
    problem: Option<Option<String>>,
    sent: Option<Vec<Deployment>>,
}

impl Out {
    fn new(tx: async_channel::Sender<EnvUpdate>) -> Out {
        Out { tx, problem: None, sent: None }
    }

    fn problem(&mut self, problem: Option<String>) {
        if self.problem.as_ref() != Some(&problem) {
            let _ = self.tx.try_send(EnvUpdate::Problem(problem.clone()));
            self.problem = Some(problem);
        }
    }

    fn deployments(&mut self, live: &BTreeMap<String, Deployment>) {
        let list: Vec<Deployment> = live.values().cloned().collect();
        if self.sent.as_ref() != Some(&list) {
            let _ = self.tx.try_send(EnvUpdate::Deployments(list.clone()));
            self.sent = Some(list);
        }
    }
}

/// The wait before trying again: [`FIRST_RETRY`], doubling up to [`LAST_RETRY`].
struct Backoff(Duration);

impl Backoff {
    fn new() -> Backoff {
        Backoff(FIRST_RETRY)
    }

    async fn wait(&mut self) {
        tokio::time::sleep(self.0).await;
        self.0 = (self.0 * 2).min(LAST_RETRY);
    }
}

#[cfg(test)]
mod tests;
