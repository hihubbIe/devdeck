//! Kubernetes objects to devdeck's cluster model.

use devdeck_core::cluster::{Container, Deployment, Image, Progress, Resources, cpu_millis, mem_mib};
use k8s_openapi::api::apps::v1 as apps;
use k8s_openapi::api::core::v1 as core;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use std::collections::BTreeMap;

/// The model of a deployment; `None` without a name or containers.
pub fn deployment(d: &apps::Deployment) -> Option<Deployment> {
    let name = d.metadata.name.clone()?;
    let spec = d.spec.as_ref()?;
    let containers: Vec<Container> = spec.template.spec.as_ref()?.containers.iter().map(container).collect();
    if containers.is_empty() {
        return None;
    }
    let status = d.status.as_ref();
    let count = |n: Option<i32>| n.unwrap_or(0).max(0) as u32;
    Some(Deployment {
        name,
        containers,
        replicas: spec.replicas.unwrap_or(1).max(0) as u32,
        ready: count(status.and_then(|s| s.ready_replicas)),
        updated: count(status.and_then(|s| s.updated_replicas)),
        progress: progress(d),
        since: since(d),
    })
}

fn container(c: &core::Container) -> Container {
    let ports = c.ports.iter().flatten().filter_map(|p| u16::try_from(p.container_port).ok()).collect();
    let mut config_maps: Vec<String> = Vec::new();
    let from = c.env_from.iter().flatten().filter_map(|e| e.config_map_ref.as_ref()).map(|r| &r.name);
    let keys = c.env.iter().flatten().filter_map(|e| e.value_from.as_ref()?.config_map_key_ref.as_ref()).map(|r| &r.name);
    for name in from.chain(keys) {
        if !config_maps.contains(name) {
            config_maps.push(name.clone());
        }
    }
    Container { name: c.name.clone(), image: Image::parse(c.image.as_deref().unwrap_or_default()), ports, resources: resources(c.resources.as_ref()), config_maps }
}

fn resources(r: Option<&core::ResourceRequirements>) -> Resources {
    fn get<'a>(map: Option<&'a BTreeMap<String, Quantity>>, key: &str) -> Option<&'a str> {
        map?.get(key).map(|q| q.0.as_str())
    }
    let requests = r.and_then(|r| r.requests.as_ref());
    let limits = r.and_then(|r| r.limits.as_ref());
    Resources {
        cpu_request_m: get(requests, "cpu").and_then(cpu_millis),
        cpu_limit_m: get(limits, "cpu").and_then(cpu_millis),
        mem_request_mi: get(requests, "memory").and_then(mem_mib),
        mem_limit_mi: get(limits, "memory").and_then(mem_mib),
    }
}

/// Paused (the spec says so), stuck (past its progress deadline), rolling (a newer spec
/// not observed yet, replicas not all updated, or the new replica set not available
/// yet), else settled.
fn progress(d: &apps::Deployment) -> Progress {
    let status = d.status.as_ref();
    let progressing = status.and_then(|s| s.conditions.as_ref()).and_then(|cs| cs.iter().find(|c| c.type_ == "Progressing"));
    let reason = progressing.and_then(|c| c.reason.as_deref());
    if d.spec.as_ref().and_then(|s| s.paused).unwrap_or(false) || reason == Some("DeploymentPaused") {
        return Progress::Paused;
    }
    if reason == Some("ProgressDeadlineExceeded") {
        return Progress::Stuck;
    }
    let replicas = d.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    let observed = status.and_then(|s| s.observed_generation).unwrap_or(0);
    let updated = status.and_then(|s| s.updated_replicas).unwrap_or(0);
    let finished = progressing.is_none() || reason == Some("NewReplicaSetAvailable");
    if observed < d.metadata.generation.unwrap_or(0) || updated < replicas || !finished { Progress::Rolling } else { Progress::Settled }
}

/// When the current version finished rolling out (the `Progressing` condition's update
/// once its new replica set is available), the creation without that condition, `None`
/// while a rollout runs.
fn since(d: &apps::Deployment) -> Option<u64> {
    if progress(d) != Progress::Settled {
        return None;
    }
    let status = d.status.as_ref();
    let seconds = |t: &k8s_openapi::apimachinery::pkg::apis::meta::v1::Time| u64::try_from(t.0.as_second()).ok();
    let progressing = status.and_then(|s| s.conditions.as_ref()).and_then(|cs| cs.iter().find(|c| c.type_ == "Progressing"));
    match progressing {
        Some(c) if c.reason.as_deref() == Some("NewReplicaSetAvailable") => c.last_update_time.as_ref().or(d.metadata.creation_timestamp.as_ref()).and_then(seconds),
        Some(_) => None,
        None => d.metadata.creation_timestamp.as_ref().and_then(seconds),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> apps::Deployment {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn epoch(rfc3339: &str) -> u64 {
        rfc3339.parse::<k8s_openapi::jiff::Timestamp>().unwrap().as_second() as u64
    }

    #[test]
    fn several_containers_keep_their_order_and_the_main_one_is_found_by_name() {
        let d = deployment(&fixture("multi-container")).unwrap();
        let names: Vec<_> = d.containers.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["envoy", "entityservice", "graphql-gateway"]);
        assert_eq!((d.main().name.as_str(), d.tag(), d.others()), ("entityservice", "lan-160".into(), 2));
        assert_eq!((d.replicas, d.ready, d.updated), (2, 2, 2));
        assert_eq!(d.since, Some(epoch("2026-10-30T14:02:11Z")), "the end of the last rollout");
        let main = d.main();
        assert_eq!(main.image.repo, "europe-west1-docker.pkg.dev/project/registry06/entityservice");
        assert_eq!(main.ports, [5800, 9090]);
        assert_eq!(main.config_maps, ["entityservice-config", "shared-env", "feature-flags"], "envFrom, then env keys, once each");
        assert_eq!(main.resources, Resources { cpu_request_m: Some(250), cpu_limit_m: Some(1000), mem_request_mi: Some(512), mem_limit_mi: Some(1024) });
        assert_eq!(d.containers[0].resources, Resources { cpu_request_m: Some(100), cpu_limit_m: None, mem_request_mi: Some(128), mem_limit_mi: Some(256) });
    }

    #[test]
    fn a_rolling_deployment_has_no_since() {
        let d = deployment(&fixture("rolling")).unwrap();
        assert_eq!((d.replicas, d.ready, d.updated, d.since), (3, 2, 1, None));
        let mut k = fixture("rolling");
        let status = k.status.as_mut().unwrap();
        status.updated_replicas = Some(3);
        assert_eq!(deployment(&k).unwrap().since, None, "still ReplicaSetUpdated");
        k.metadata.generation = Some(8);
        assert_eq!(deployment(&k).unwrap().since, None, "a newer spec not observed yet");
        assert_eq!(deployment(&k).unwrap().progress, Progress::Rolling);
    }

    #[test]
    fn paused_and_stuck_rollouts_say_so() {
        let mut k = fixture("rolling");
        k.spec.as_mut().unwrap().paused = Some(true);
        assert_eq!((deployment(&k).unwrap().progress, deployment(&k).unwrap().since), (Progress::Paused, None));
        let mut k = fixture("rolling");
        let conditions = k.status.as_mut().unwrap().conditions.as_mut().unwrap();
        let progressing = conditions.iter_mut().find(|c| c.type_ == "Progressing").unwrap();
        progressing.status = "False".into();
        progressing.reason = Some("ProgressDeadlineExceeded".into());
        assert_eq!(deployment(&k).unwrap().progress, Progress::Stuck);
        assert_eq!(deployment(&fixture("multi-container")).unwrap().progress, Progress::Settled);
    }

    #[test]
    fn without_conditions_since_is_the_creation() {
        let d = deployment(&fixture("no-conditions")).unwrap();
        assert_eq!(d.since, Some(epoch("2026-09-01T08:00:00Z")));
        assert_eq!((d.replicas, d.ready, d.updated), (1, 1, 1));
    }

    #[test]
    fn unset_requests_and_limits_stay_unset() {
        let d = deployment(&fixture("no-resources")).unwrap();
        assert_eq!(d.containers[0].resources, Resources::default());
        assert_eq!(d.containers[1].resources, Resources { cpu_request_m: Some(50), cpu_limit_m: None, mem_request_mi: None, mem_limit_mi: None });
        assert!(d.containers[0].ports.is_empty() && d.containers[0].config_maps.is_empty());
    }

    #[test]
    fn a_digest_image_has_no_tag() {
        let d = deployment(&fixture("digest")).unwrap();
        let image = &d.main().image;
        assert_eq!((image.tag.as_deref(), image.digest.as_deref()), (None, Some("sha256:4f1c2b7a9d8e6f5a3c2b1a0f9e8d7c6b5a4f3e2d1c0b9a8f7e6d5c4b3a2f1e0d")));
        assert_eq!(d.tag(), "@sha256:4f1c2b7a9d8e");
    }

    #[test]
    fn nameless_or_empty_deployments_are_skipped() {
        let mut k = fixture("no-conditions");
        k.spec.as_mut().unwrap().template.spec.as_mut().unwrap().containers.clear();
        assert_eq!(deployment(&k), None);
        let mut k = fixture("no-conditions");
        k.metadata.name = None;
        assert_eq!(deployment(&k), None);
    }
}
