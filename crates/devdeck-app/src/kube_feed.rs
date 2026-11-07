//! Read-only Kubernetes feed for the spike: a tokio runtime on its own thread lists
//! deployments every 15 s and sends the result over a channel. Nothing about the
//! cluster is hard-coded (R1.5): it comes from environment variables until M2 config.

use k8s_openapi::api::apps::v1::Deployment;
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Api, Client, Config, api::ListParams};
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum KubeState {
    NotConfigured,
    Loading,
    Error(String),
    Ready(Vec<DeploymentRow>),
}

#[derive(Clone, Debug)]
pub struct DeploymentRow {
    pub name: String,
    pub tag: String,
    pub ready: i32,
    pub desired: i32,
}

struct Target {
    kubeconfig: String,
    context: Option<String>,
    namespace: String,
}

fn target() -> Option<Target> {
    Some(Target {
        kubeconfig: std::env::var("DEVDECK_KUBECONFIG").ok()?,
        context: std::env::var("DEVDECK_KUBE_CONTEXT").ok(),
        namespace: std::env::var("DEVDECK_KUBE_NAMESPACE").unwrap_or_else(|_| "default".into()),
    })
}

/// Starts the feed if configured. The thread ends when the receiver is dropped.
pub fn start() -> Option<async_channel::Receiver<KubeState>> {
    let target = target()?;
    let (tx, rx) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("kube-feed".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send_blocking(KubeState::Error(format!("runtime: {e}")));
                    return;
                }
            };
            rt.block_on(async move {
                loop {
                    let state = match list(&target).await {
                        Ok(rows) => KubeState::Ready(rows),
                        Err(e) => KubeState::Error(e),
                    };
                    if tx.send(state).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(15)).await;
                    if tx.is_closed() {
                        break;
                    }
                }
            });
        })
        .ok()?;
    Some(rx)
}

async fn list(t: &Target) -> Result<Vec<DeploymentRow>, String> {
    let kc = Kubeconfig::read_from(&t.kubeconfig).map_err(|e| format!("kubeconfig {}: {e}", t.kubeconfig))?;
    let opts = KubeConfigOptions { context: t.context.clone(), ..Default::default() };
    let config = Config::from_custom_kubeconfig(kc, &opts).await.map_err(|e| format!("config: {e}"))?;
    let client = Client::try_from(config).map_err(|e| format!("client: {e}"))?;
    let api: Api<Deployment> = Api::namespaced(client, &t.namespace);
    let list = api.list(&ListParams::default()).await.map_err(|e| match e {
        kube::Error::Api(s) if s.code == 403 => format!("no permission to list deployments in {} (403)", t.namespace),
        other => other.to_string(),
    })?;
    let mut rows: Vec<DeploymentRow> = list
        .items
        .into_iter()
        .map(|d| {
            let name = d.metadata.name.unwrap_or_default();
            let image = d
                .spec
                .as_ref()
                .and_then(|s| s.template.spec.as_ref())
                .and_then(|p| p.containers.first())
                .and_then(|c| c.image.clone())
                .unwrap_or_default();
            let tag = image.rsplit_once(':').map(|(_, t)| t.to_string()).unwrap_or(image);
            let desired = d.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0);
            let ready = d.status.as_ref().and_then(|s| s.ready_replicas).unwrap_or(0);
            DeploymentRow { name, tag, ready, desired }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows)
}
