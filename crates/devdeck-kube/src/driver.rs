//! Live watches of an env's deployments or pods, and reads of its pods' metrics, on one
//! tokio runtime for the whole process, started by the first use (a workspace without a
//! cluster doesn't pay for it).
//!
//! - One client per kubeconfig file, cluster and user, shared by the envs of the file,
//!   built again when the file's content changes and dropped with its last watch.
//! - A watch follows kube-runtime's watcher on the env's namespace (pods optionally
//!   filtered by a label selector): a list (`Init` … `InitDone`) replaces what was known,
//!   so objects deleted while the watch was down disappear; then each change updates it.
//!   The objects are sent [`SETTLE`] after the first change not sent yet, and only when
//!   their models differ from what was sent last (a status that moves without changing
//!   the model sends nothing).
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
use devdeck_core::cluster::{Deployment, Pod, Usage, cpu_millis, mem_mib};
use futures_lite::StreamExt;
use k8s_openapi::NamespaceResourceScope;
use k8s_openapi::api::apps::v1 as apps;
use k8s_openapi::api::core::v1 as core;
use k8s_openapi::serde::de::DeserializeOwned;
use kube::api::{ApiResource, DynamicObject, GroupVersionKind, ListParams};
use kube::config::KubeConfigOptions;
use kube::runtime::watcher::{self, Event, watcher};
use kube::{Api, Client, Resource, ResourceExt};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;

/// How long after a first change the deployments are sent: a rollout touching many fields
/// is one update, and steady churn still sends at this pace.
const SETTLE: Duration = Duration::from_millis(100);
/// How long a metrics read may take.
const METRICS_TIMEOUT: Duration = Duration::from_secs(10);
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

/// What a pod watch sends.
#[derive(Clone, Debug, PartialEq)]
pub enum PodUpdate {
    /// Every pod watched, by name, sent when it changed.
    Pods(Vec<Pod>),
    /// What's wrong now (`None`: nothing any more).
    Problem(Option<String>),
}

/// Why pod metrics couldn't be read.
#[derive(Clone, Debug, PartialEq)]
pub enum MetricsError {
    /// They can't be read in this env, not now nor later (no permission, no metrics API).
    Refused(String),
    /// This read failed.
    Failed(String),
}

impl std::fmt::Display for MetricsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetricsError::Refused(m) | MetricsError::Failed(m) => f.write_str(m),
        }
    }
}

/// A kind of object a watch follows, its model, and what the watch sends.
pub(crate) trait Watched: Resource<DynamicType = (), Scope = NamespaceResourceScope> + Clone + DeserializeOwned + Debug + Send + Sync + 'static {
    /// What a watch does, for messages (`list deployments`).
    const DOING: &'static str;
    type Model: Clone + PartialEq + Send + 'static;
    type Update: Send + 'static;
    /// The model; `None` for an object left out.
    fn model(&self) -> Option<Self::Model>;
    fn list(models: Vec<Self::Model>) -> Self::Update;
    fn problem(problem: Option<String>) -> Self::Update;
}

impl Watched for apps::Deployment {
    const DOING: &'static str = "list deployments";
    type Model = Deployment;
    type Update = EnvUpdate;

    fn model(&self) -> Option<Deployment> {
        convert::deployment(self)
    }

    fn list(models: Vec<Deployment>) -> EnvUpdate {
        EnvUpdate::Deployments(models)
    }

    fn problem(problem: Option<String>) -> EnvUpdate {
        EnvUpdate::Problem(problem)
    }
}

impl Watched for core::Pod {
    const DOING: &'static str = "list pods";
    type Model = Pod;
    type Update = PodUpdate;

    fn model(&self) -> Option<Pod> {
        convert::pod(self)
    }

    fn list(models: Vec<Pod>) -> PodUpdate {
        PodUpdate::Pods(models)
    }

    fn problem(problem: Option<String>) -> PodUpdate {
        PodUpdate::Problem(problem)
    }
}

/// Dropping it stops the watch.
pub struct WatchHandle {
    pub(crate) task: tokio::task::AbortHandle,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Starts watching `target`'s deployments. The watch also ends when the receiver is
/// dropped.
pub fn watch(target: &Target) -> (async_channel::Receiver<EnvUpdate>, WatchHandle) {
    start::<apps::Deployment>(target, watcher::Config::default())
}

/// Starts watching `target`'s pods, those `selector` selects (a label selector, as
/// [`Deployment::selector`]) or every one. The watch also ends when the receiver is
/// dropped.
pub fn watch_pods(target: &Target, selector: Option<&str>) -> (async_channel::Receiver<PodUpdate>, WatchHandle) {
    start::<core::Pod>(target, selecting(selector))
}

/// A watcher's config for the objects `selector` selects (all without one, or an empty one).
fn selecting(selector: Option<&str>) -> watcher::Config {
    match selector.filter(|s| !s.is_empty()) {
        Some(selector) => watcher::Config::default().labels(selector),
        None => watcher::Config::default(),
    }
}

fn start<K: Watched>(target: &Target, config: watcher::Config) -> (async_channel::Receiver<K::Update>, WatchHandle) {
    let (tx, rx) = async_channel::unbounded();
    let target = target.clone();
    let mut out = Out::<K>::new(tx.clone());
    let task = runtime().spawn(until_closed(tx, async move {
        let (client, _user) = connect(&target, &mut out).await;
        feed(client, &target, config, &mut out).await;
    }));
    (rx, WatchHandle { task: task.abort_handle() })
}

/// Reads the usage of `target`'s pods once (`metrics.k8s.io`), by pod: the sum of its
/// containers'. The result comes on the channel, which closes after it.
pub fn metrics(target: &Target) -> async_channel::Receiver<Result<BTreeMap<String, Usage>, MetricsError>> {
    let (tx, rx) = async_channel::bounded(1);
    let target = target.clone();
    let sender = tx.clone();
    runtime().spawn(until_closed(tx, async move {
        let read = async {
            match client(&target).await {
                Ok((client, _user)) => read_metrics(client, &target).await,
                Err(e) => Err(MetricsError::Failed(describe(&e, &target, "list pod metrics"))),
            }
        };
        // A connection that died without a word would never answer: the next read retries.
        let result = tokio::time::timeout(METRICS_TIMEOUT, read).await.unwrap_or_else(|_| Err(MetricsError::Failed(format!("{} ({}) didn't answer the metrics read", target.cluster, target.env))));
        let _ = sender.try_send(result);
    }));
    rx
}

/// One list of `target`'s pod metrics through `client`.
async fn read_metrics(client: Client, target: &Target) -> Result<BTreeMap<String, Usage>, MetricsError> {
    let resource = ApiResource::from_gvk_with_plural(&GroupVersionKind::gvk("metrics.k8s.io", "v1beta1", "PodMetrics"), "pods");
    let api: Api<DynamicObject> = Api::namespaced_with(client, &target.namespace, &resource);
    let list = api.list(&ListParams::default()).await.map_err(|e| match &e {
        kube::Error::Api(status) if status.code == 403 => MetricsError::Refused(format!("your account can't read metrics in {}", target.env)),
        kube::Error::Api(status) if status.code == 404 => MetricsError::Refused(format!("metrics aren't available on {}", target.cluster)),
        _ => MetricsError::Failed(describe(&e, target, "list pod metrics")),
    })?;
    let mut usage = BTreeMap::new();
    for pod in list.items {
        let mut sum = Usage { cpu_m: 0, mem_mi: 0 };
        for container in pod.data["containers"].as_array().into_iter().flatten() {
            let usage = &container["usage"];
            sum.cpu_m += usage["cpu"].as_str().and_then(cpu_millis).unwrap_or(0);
            sum.mem_mi += usage["memory"].as_str().and_then(mem_mib).unwrap_or(0);
        }
        usage.insert(pod.name_any(), sum);
    }
    Ok(usage)
}

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
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
pub(crate) async fn until_closed<T>(tx: async_channel::Sender<T>, work: impl Future<Output = ()>) {
    futures_lite::future::or(work, async move { tx.closed().await }).await
}

/// The client for `target`, retried with the backoff while it can't be built.
async fn connect<K: Watched>(target: &Target, out: &mut Out<K>) -> (Client, ClientUser) {
    let mut backoff = Backoff::new();
    loop {
        match client(target).await {
            Ok(client) => return client,
            Err(e) => {
                out.problem(Some(describe(&e, target, K::DOING)));
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

/// A use of a client (a watch, a metrics read): the last one dropped drops the client.
pub(crate) struct ClientUser(ClientKey);

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
pub(crate) async fn client(target: &Target) -> Result<(Client, ClientUser), kube::Error> {
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

/// Sends the models of `target`'s objects `config` selects, read through `client`, until
/// dropped.
async fn feed<K: Watched>(client: Client, target: &Target, config: watcher::Config, out: &mut Out<K>) {
    let api: Api<K> = Api::namespaced(client, &target.namespace);
    let config = config.timeout(WATCH_TIMEOUT);
    let mut live: BTreeMap<String, K::Model> = BTreeMap::new();
    let mut backoff = Backoff::new();
    loop {
        let failure = {
            let mut stream = std::pin::pin!(watcher(api.clone(), config.clone()));
            let mut listing: Option<BTreeMap<String, K::Model>> = None;
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
                            out.list(&live);
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
                out.list(&live);
            }
            failure
        };
        match failure {
            // An expired resource version: the next list is the fix, not a problem.
            Some(watcher::Error::WatchError(status)) if status.code == 410 => {}
            Some(e) => out.problem(Some(describe_watcher(&e, target, K::DOING))),
            None => {}
        }
        backoff.wait().await;
    }
}

/// Keeps `object`'s model under its name, or drops it when it has none.
fn apply<K: Watched>(map: &mut BTreeMap<String, K::Model>, object: &K) {
    match object.model() {
        Some(model) => {
            map.insert(object.name_any(), model);
        }
        None => {
            map.remove(&object.name_any());
        }
    }
}

/// The sending end of a watch, which sends only what changed.
struct Out<K: Watched> {
    tx: async_channel::Sender<K::Update>,
    problem: Option<Option<String>>,
    sent: Option<Vec<K::Model>>,
}

impl<K: Watched> Out<K> {
    fn new(tx: async_channel::Sender<K::Update>) -> Out<K> {
        Out { tx, problem: None, sent: None }
    }

    fn problem(&mut self, problem: Option<String>) {
        if self.problem.as_ref() != Some(&problem) {
            let _ = self.tx.try_send(K::problem(problem.clone()));
            self.problem = Some(problem);
        }
    }

    /// Sends the models, by name, when they changed.
    fn list(&mut self, live: &BTreeMap<String, K::Model>) {
        let list: Vec<K::Model> = live.values().cloned().collect();
        if self.sent.as_ref() != Some(&list) {
            let _ = self.tx.try_send(K::list(list.clone()));
            self.sent = Some(list);
        }
    }
}

/// The wait before trying again: [`FIRST_RETRY`], doubling up to [`LAST_RETRY`].
pub(crate) struct Backoff(Duration);

impl Backoff {
    pub(crate) fn new() -> Backoff {
        Backoff(FIRST_RETRY)
    }

    pub(crate) async fn wait(&mut self) {
        tokio::time::sleep(self.0).await;
        self.0 = (self.0 * 2).min(LAST_RETRY);
    }
}

#[cfg(test)]
mod tests;
