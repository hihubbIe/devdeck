//! Kubernetes objects to devdeck's cluster model.

use devdeck_core::cluster::{Container, ContainerStatus, Deployment, Image, Pod, PodPhase, Progress, Resources, Termination, cpu_millis, mem_mib};
use k8s_openapi::api::apps::v1 as apps;
use k8s_openapi::api::core::v1 as core;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, Time};
use kube::core::Selector;
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
        containers,
        replicas: spec.replicas.unwrap_or(1).max(0) as u32,
        ready: count(status.and_then(|s| s.ready_replicas)),
        updated: count(status.and_then(|s| s.updated_replicas)),
        progress: progress(d),
        selector: selector(&spec.selector),
        since: since(d),
        generation: d.metadata.generation.unwrap_or(0).max(0) as u64,
        observed: status.and_then(|s| s.observed_generation).unwrap_or(0).max(0) as u64,
        revision: d.metadata.annotations.as_ref().and_then(|a| a.get("deployment.kubernetes.io/revision")).and_then(|r| r.trim().parse().ok()).unwrap_or(0),
        template: newest_template(d, &name),
        failure: condition(d, "ReplicaFailure").filter(|c| c.status == "True").and_then(|c| words(&c.message).or(words(&c.reason))),
        manager: manager(d),
        template_annotated: spec.template.metadata.as_ref().and_then(|m| m.annotations.as_ref()).is_some_and(|a| !a.is_empty()),
        restarted_at: spec.template.metadata.as_ref().and_then(|m| m.annotations.as_ref()).and_then(|a| a.get(RESTARTED_AT)).cloned(),
        name,
    })
}

/// The pod template annotation `kubectl rollout restart` sets, and devdeck's redeploy.
pub const RESTARTED_AT: &str = "kubectl.kubernetes.io/restartedAt";

fn condition<'a>(d: &'a apps::Deployment, type_: &str) -> Option<&'a apps::DeploymentCondition> {
    d.status.as_ref()?.conditions.as_ref()?.iter().find(|c| c.type_ == type_)
}

/// A text that says something, trimmed.
fn words(text: &Option<String>) -> Option<String> {
    text.as_deref().map(str::trim).filter(|t| !t.is_empty()).map(String::from)
}

/// The newest pod template's hash: the first quoted replica set of `name` in the
/// `Progressing` condition's message (`ReplicaSet "<name>-<hash>" is progressing.`, `Created
/// new replica set "<name>-<hash>"`).
fn newest_template(d: &apps::Deployment, name: &str) -> Option<String> {
    let message = condition(d, "Progressing")?.message.as_deref()?;
    let prefix = format!("{name}-");
    message.split('"').skip(1).step_by(2).find_map(|quoted| quoted.strip_prefix(&prefix)).filter(|hash| !hash.is_empty() && !hash.contains(char::is_whitespace)).map(String::from)
}

/// Who changed the spec last: the manager of the newest `managedFields` entry (by its
/// time) that touches `spec` and isn't the `status` subresource's.
fn manager(d: &apps::Deployment) -> Option<String> {
    let entries = d.metadata.managed_fields.as_deref().unwrap_or_default();
    entries
        .iter()
        .filter(|e| e.subresource.as_deref() != Some("status"))
        // The pod template: who deployed, not who scaled.
        .filter(|e| e.fields_v1.as_ref().is_some_and(|f| f.0.get("f:spec").and_then(|s| s.get("f:template")).is_some()))
        .filter(|e| e.manager.as_deref().is_some_and(|m| !m.is_empty()))
        .max_by_key(|e| e.time.as_ref().map(|t| t.0))
        .and_then(|e| e.manager.clone())
}

/// A label selector as the API takes it (`app=api,tier in (web)`); empty when it selects
/// every pod or can't be read.
fn selector(s: &LabelSelector) -> String {
    Selector::try_from(s.clone()).ok().filter(|s| !s.selects_all()).map(|s| s.to_string()).unwrap_or_default()
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
    Container {
        name: c.name.clone(),
        image: Image::parse(c.image.as_deref().unwrap_or_default()),
        ports,
        resources: resources(c.resources.as_ref()),
        config_maps,
        pull_policy: c.image_pull_policy.clone().filter(|p| !p.is_empty()),
    }
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

/// Paused (the spec says so), rolling while a newer spec isn't observed yet (its status
/// and conditions are about the spec before: a deploy over a stuck rollout rolls), stuck
/// (past its progress deadline), rolling (replicas not all updated, or the new replica
/// set not available yet), else settled.
fn progress(d: &apps::Deployment) -> Progress {
    let status = d.status.as_ref();
    let progressing = condition(d, "Progressing");
    let reason = progressing.and_then(|c| c.reason.as_deref());
    if d.spec.as_ref().and_then(|s| s.paused).unwrap_or(false) || reason == Some("DeploymentPaused") {
        return Progress::Paused;
    }
    let observed = status.and_then(|s| s.observed_generation).unwrap_or(0);
    if observed < d.metadata.generation.unwrap_or(0) {
        return Progress::Rolling;
    }
    if reason == Some("ProgressDeadlineExceeded") {
        return Progress::Stuck;
    }
    let replicas = d.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1);
    let updated = status.and_then(|s| s.updated_replicas).unwrap_or(0);
    let finished = progressing.is_none() || reason == Some("NewReplicaSetAvailable");
    if updated < replicas || !finished { Progress::Rolling } else { Progress::Settled }
}

/// When the current version finished rolling out (the `Progressing` condition's update
/// once its new replica set is available), the creation without that condition, `None`
/// while a rollout runs.
fn since(d: &apps::Deployment) -> Option<u64> {
    if progress(d) != Progress::Settled {
        return None;
    }
    match condition(d, "Progressing") {
        Some(c) if c.reason.as_deref() == Some("NewReplicaSetAvailable") => c.last_update_time.as_ref().or(d.metadata.creation_timestamp.as_ref()).and_then(seconds),
        Some(_) => None,
        None => d.metadata.creation_timestamp.as_ref().and_then(seconds),
    }
}

fn seconds(t: &Time) -> Option<u64> {
    u64::try_from(t.0.as_second()).ok()
}

/// The model of a pod; `None` without a name.
pub fn pod(p: &core::Pod) -> Option<Pod> {
    let name = p.metadata.name.clone()?;
    let template = p.metadata.labels.as_ref().and_then(|l| l.get("pod-template-hash")).cloned().unwrap_or_default();
    let owner = p.metadata.owner_references.iter().flatten().find(|r| r.kind == "ReplicaSet").and_then(|r| {
        let suffix = format!("-{template}");
        r.name.strip_suffix(&suffix).filter(|_| !template.is_empty()).map(String::from)
    });
    let status = p.status.as_ref();
    let init = status.and_then(|s| s.init_container_statuses.as_deref()).unwrap_or_default();
    let main = status.and_then(|s| s.container_statuses.as_deref()).unwrap_or_default();
    let containers: Vec<ContainerStatus> = init.iter().map(|c| container_status(c, true)).chain(main.iter().map(|c| container_status(c, false))).collect();
    let ready = status.and_then(|s| s.conditions.as_ref()).is_some_and(|cs| cs.iter().any(|c| c.type_ == "Ready" && c.status == "True"));
    let last_termination = containers.iter().filter_map(|c| c.last_termination.clone()).max_by_key(|t| t.at);
    let spec = p.spec.as_ref();
    let unschedulable = status.and_then(|s| s.conditions.as_ref()).and_then(|cs| cs.iter().find(|c| c.type_ == "PodScheduled" && c.status == "False")).and_then(|c| words(&c.message).or(words(&c.reason)));
    let image = spec.and_then(|s| s.containers.iter().find(|c| Some(&c.name) == owner.as_ref()).or(s.containers.first())).and_then(|c| c.image.as_deref());
    Some(Pod {
        phase: phase(p, init, main),
        ready,
        restarts: containers.iter().map(|c| c.restarts).sum(),
        last_termination,
        started: status.and_then(|s| s.start_time.as_ref()).and_then(seconds),
        node: spec.and_then(|s| s.node_name.clone()).unwrap_or_default(),
        tag: image.map(|i| Image::parse(i).label()).unwrap_or_default(),
        containers,
        name,
        owner,
        template,
        unschedulable,
    })
}

/// Terminating once deleted, failed or succeeded as its phase says, else waiting when a
/// container waits (an init container first), else running or pending.
fn phase(p: &core::Pod, init: &[core::ContainerStatus], main: &[core::ContainerStatus]) -> PodPhase {
    if p.metadata.deletion_timestamp.is_some() {
        return PodPhase::Terminating;
    }
    let status = p.status.as_ref();
    let phase = status.and_then(|s| s.phase.as_deref());
    match phase {
        Some("Failed") => return PodPhase::Failed(status.and_then(|s| s.reason.clone()).unwrap_or_else(|| "Failed".into())),
        Some("Succeeded") => return PodPhase::Succeeded,
        _ => {}
    }
    let waiting = |c: &core::ContainerStatus| c.state.as_ref()?.waiting.as_ref()?.reason.clone().filter(|r| !r.is_empty());
    if let Some(reason) = init.iter().find_map(waiting) {
        return PodPhase::Waiting(format!("Init:{reason}"));
    }
    if let Some(reason) = main.iter().find_map(waiting) {
        return PodPhase::Waiting(reason);
    }
    if phase == Some("Running") { PodPhase::Running } else { PodPhase::Pending }
}

fn container_status(c: &core::ContainerStatus, init: bool) -> ContainerStatus {
    let state = c.state.as_ref();
    let text = match state {
        Some(s) if s.running.is_some() => "running".into(),
        Some(core::ContainerState { waiting: Some(w), .. }) => match w.reason.as_deref().filter(|r| !r.is_empty()) {
            Some(reason) => format!("waiting: {reason}"),
            None => "waiting".into(),
        },
        Some(core::ContainerState { terminated: Some(t), .. }) => format!("terminated: {} (exit {})", termination(t).reason, t.exit_code),
        _ => "unknown".into(),
    };
    // A container that ended well (a finished init container) didn't fail: only an exit
    // other than 0 counts, else its previous end.
    let failed = state.and_then(|s| s.terminated.as_ref()).filter(|t| t.exit_code != 0);
    let ended = failed.or(c.last_state.as_ref().and_then(|s| s.terminated.as_ref()));
    let message = state.and_then(|s| s.waiting.as_ref()).and_then(|w| words(&w.message));
    ContainerStatus { name: c.name.clone(), init, ready: c.ready, restarts: c.restart_count.max(0) as u32, state: text, message, last_termination: ended.map(termination) }
}

fn termination(t: &core::ContainerStateTerminated) -> Termination {
    let reason = t.reason.clone().filter(|r| !r.is_empty()).unwrap_or_else(|| if t.exit_code == 0 { "Completed".into() } else { "Error".into() });
    let message = t.message.as_deref().and_then(|m| m.lines().map(str::trim).find(|l| !l.is_empty())).map(String::from);
    Termination { reason, exit_code: Some(t.exit_code), message, at: t.finished_at.as_ref().and_then(seconds) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read<T: k8s_openapi::serde::de::DeserializeOwned>(name: &str) -> T {
        let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    fn fixture(name: &str) -> apps::Deployment {
        read(name)
    }

    /// The model of fixture `pod-<name>`.
    fn pod_fixture(name: &str) -> Pod {
        pod(&read(&format!("pod-{name}"))).unwrap()
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
    fn a_new_spec_rolls_before_a_stuck_rollout_counts() {
        let mut k = fixture("rolling");
        let progressing = k.status.as_mut().unwrap().conditions.as_mut().unwrap().iter_mut().find(|c| c.type_ == "Progressing").unwrap();
        progressing.status = "False".into();
        progressing.reason = Some("ProgressDeadlineExceeded".into());
        k.metadata.generation = Some(8);
        let d = deployment(&k).unwrap();
        assert_eq!((d.progress, d.generation, d.observed), (Progress::Rolling, 8, 7), "a redeploy over a stuck rollout, not observed yet");
        k.status.as_mut().unwrap().observed_generation = Some(8);
        assert_eq!(deployment(&k).unwrap().progress, Progress::Stuck, "observed, and still past its deadline");
        k.spec.as_mut().unwrap().paused = Some(true);
        k.metadata.generation = Some(9);
        assert_eq!(deployment(&k).unwrap().progress, Progress::Paused, "paused comes first");
    }

    #[test]
    fn generations_revision_and_the_newest_template() {
        let d = deployment(&fixture("rolling")).unwrap();
        assert_eq!((d.generation, d.observed, d.revision, d.template.as_deref()), (7, 7, 18, Some("5f6d7c8b9")));
        assert_eq!((d.failure, d.manager), (None, None));
        let d = deployment(&fixture("multi-container")).unwrap();
        assert_eq!((d.generation, d.revision, d.template.as_deref()), (14, 42, Some("7d9c8b6f5")), "has successfully progressed");
        let d = deployment(&fixture("no-conditions")).unwrap();
        assert_eq!((d.revision, d.template), (1, None), "no Progressing condition");
    }

    #[test]
    fn the_pull_policy_and_the_templates_annotations() {
        let mut k = fixture("rolling");
        let d = deployment(&k).unwrap();
        assert_eq!((d.main().pull_policy.as_deref(), d.template_annotated, d.restarted_at.as_deref()), (Some("Always"), false, None));
        let template = &mut k.spec.as_mut().unwrap().template;
        template.spec.as_mut().unwrap().containers[0].image_pull_policy = Some("IfNotPresent".into());
        let meta = template.metadata.get_or_insert_with(Default::default);
        meta.annotations = Some(BTreeMap::new());
        let d = deployment(&k).unwrap();
        assert_eq!((d.main().pull_policy.as_deref(), d.template_annotated), (Some("IfNotPresent"), false), "an empty map is no annotations");
        k.spec.as_mut().unwrap().template.spec.as_mut().unwrap().containers[0].image_pull_policy = None;
        assert_eq!(deployment(&k).unwrap().main().pull_policy, None, "unset: the API server's default applies");
        let annotations = k.spec.as_mut().unwrap().template.metadata.as_mut().unwrap().annotations.as_mut().unwrap();
        annotations.insert(RESTARTED_AT.into(), "2026-11-10T09:12:44.512Z".into());
        let d = deployment(&k).unwrap();
        assert_eq!((d.template_annotated, d.restarted_at.as_deref()), (true, Some("2026-11-10T09:12:44.512Z")));
    }

    #[test]
    fn a_missing_or_bad_revision_is_0() {
        let mut k = fixture("rolling");
        k.metadata.annotations.as_mut().unwrap().insert("deployment.kubernetes.io/revision".into(), "eighteen".into());
        assert_eq!(deployment(&k).unwrap().revision, 0);
        k.metadata.annotations = None;
        assert_eq!(deployment(&k).unwrap().revision, 0);
    }

    #[test]
    fn the_template_is_a_replica_set_of_this_deployment() {
        let with = |message: &str| {
            let mut k = fixture("rolling");
            let progressing = k.status.as_mut().unwrap().conditions.as_mut().unwrap().iter_mut().find(|c| c.type_ == "Progressing").unwrap();
            progressing.message = Some(message.into());
            deployment(&k).unwrap().template
        };
        assert_eq!(with("Created new replica set \"graphservice-7a8b9c0d1\"").as_deref(), Some("7a8b9c0d1"));
        assert_eq!(with("ReplicaSet \"graphservice-5f6d7c8b9\" has timed out progressing.").as_deref(), Some("5f6d7c8b9"));
        assert_eq!(with("ReplicaSet \"graphservice-api-5f6d7c8b9\" is progressing.").as_deref(), Some("api-5f6d7c8b9"), "only the name's prefix is cut");
        assert_eq!(with("ReplicaSet \"billing-5f6d7c8b9\" is progressing."), None, "another deployment's");
        assert_eq!(with("ReplicaSet \"graphservice-\" is progressing."), None, "no hash");
        assert_eq!(with("Deployment is paused"), None);
    }

    #[test]
    fn a_replica_failure_says_why() {
        let mut k = fixture("rolling");
        let failure = serde_json::json!({
            "type": "ReplicaFailure", "status": "True", "reason": "FailedCreate",
            "message": "pods \"graphservice-5f6d7c8b9-abcde\" is forbidden: exceeded quota: compute, requested: cpu=500m, used: cpu=4, limited: cpu=4"
        });
        k.status.as_mut().unwrap().conditions.as_mut().unwrap().push(serde_json::from_value(failure).unwrap());
        assert_eq!(deployment(&k).unwrap().failure.as_deref(), Some("pods \"graphservice-5f6d7c8b9-abcde\" is forbidden: exceeded quota: compute, requested: cpu=500m, used: cpu=4, limited: cpu=4"));
        fn replica_failure(k: &mut apps::Deployment) -> &mut apps::DeploymentCondition {
            k.status.as_mut().unwrap().conditions.as_mut().unwrap().iter_mut().find(|c| c.type_ == "ReplicaFailure").unwrap()
        }
        replica_failure(&mut k).message = Some(" ".into());
        assert_eq!(deployment(&k).unwrap().failure.as_deref(), Some("FailedCreate"), "the reason without a message");
        replica_failure(&mut k).status = "False".into();
        assert_eq!(deployment(&k).unwrap().failure, None, "over");
    }

    #[test]
    fn the_manager_is_the_newest_to_touch_the_spec() {
        let mut k = fixture("rolling");
        let fields = serde_json::json!([
            {"manager": "helm", "operation": "Update", "apiVersion": "apps/v1", "time": "2026-11-01T10:00:00Z", "fieldsType": "FieldsV1",
             "fieldsV1": {"f:metadata": {"f:labels": {}}, "f:spec": {"f:replicas": {}, "f:template": {}}}},
            {"manager": "kubectl-set", "operation": "Update", "apiVersion": "apps/v1", "time": "2026-11-07T10:30:00Z", "fieldsType": "FieldsV1",
             "fieldsV1": {"f:spec": {"f:template": {"f:spec": {"f:containers": {}}}}}},
            {"manager": "kube-controller-manager", "operation": "Update", "apiVersion": "apps/v1", "time": "2026-11-07T10:31:52Z", "fieldsType": "FieldsV1", "subresource": "status",
             "fieldsV1": {"f:status": {"f:replicas": {}}, "f:spec": {}}},
            {"manager": "argocd", "operation": "Update", "apiVersion": "apps/v1", "time": "2026-11-08T09:00:00Z", "fieldsType": "FieldsV1",
             "fieldsV1": {"f:metadata": {"f:annotations": {}}}}
        ]);
        k.metadata.managed_fields = Some(serde_json::from_value(fields).unwrap());
        assert_eq!(deployment(&k).unwrap().manager.as_deref(), Some("kubectl-set"), "not the status subresource, not a metadata-only entry");
        let devdeck = serde_json::json!({"manager": "devdeck", "operation": "Update", "apiVersion": "apps/v1", "time": "2026-11-09T12:00:00Z", "fieldsType": "FieldsV1",
            "fieldsV1": {"f:spec": {"f:template": {"f:spec": {"f:containers": {}}}}}});
        k.metadata.managed_fields.as_mut().unwrap().insert(0, serde_json::from_value(devdeck).unwrap());
        assert_eq!(deployment(&k).unwrap().manager.as_deref(), Some("devdeck"), "by time, not by place");
        k.metadata.managed_fields = None;
        assert_eq!(deployment(&k).unwrap().manager, None);
    }

    #[test]
    fn a_waiting_container_keeps_its_message() {
        let p = pod_fixture("image-pull");
        assert_eq!(p.containers[0].message.as_deref(), Some("Back-off pulling image \"europe-west1-docker.pkg.dev/project/registry06/searchservice:lan-999\""));
        let p = pod_fixture("crash-loop");
        assert_eq!(p.containers[0].message.as_deref(), Some("back-off 5m0s restarting failed container=entityservice pod=entityservice-7b5d9f6c8-q8r2t_dev35(a4c2e1f0)"));
        assert_eq!(pod_fixture("running").containers[0].message, None, "running");
        let mut k: core::Pod = read("pod-image-pull");
        let status = &mut k.status.as_mut().unwrap().container_statuses.as_mut().unwrap()[0];
        status.state.as_mut().unwrap().waiting.as_mut().unwrap().message = Some(String::new());
        assert_eq!(pod(&k).unwrap().containers[0].message, None, "an empty message");
    }

    #[test]
    fn an_unschedulable_pod_says_why() {
        assert_eq!(pod_fixture("pending").unschedulable.as_deref(), Some("0/6 nodes are available: 6 Insufficient cpu."));
        let mut k: core::Pod = read("pod-pending");
        let scheduled = &mut k.status.as_mut().unwrap().conditions.as_mut().unwrap()[0];
        scheduled.message = None;
        assert_eq!(pod(&k).unwrap().unschedulable.as_deref(), Some("Unschedulable"), "its reason without a message");
        k.status.as_mut().unwrap().conditions.as_mut().unwrap()[0].status = "True".into();
        assert_eq!(pod(&k).unwrap().unschedulable, None, "scheduled");
        assert_eq!(pod_fixture("running").unschedulable, None);
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

    #[test]
    fn the_selector_is_the_whole_label_selector() {
        assert_eq!(deployment(&fixture("multi-container")).unwrap().selector, "app=entityservice");
        let mut k = fixture("rolling");
        let selector = serde_json::json!({"matchLabels": {"app": "graphservice", "tier": "web"}, "matchExpressions": [{"key": "track", "operator": "NotIn", "values": ["canary"]}, {"key": "team", "operator": "Exists"}]});
        k.spec.as_mut().unwrap().selector = serde_json::from_value(selector).unwrap();
        assert_eq!(deployment(&k).unwrap().selector, "app=graphservice,tier=web,track notin (canary),team");
        k.spec.as_mut().unwrap().selector = Default::default();
        assert_eq!(deployment(&k).unwrap().selector, "", "selects every pod");
        let unknown = serde_json::json!({"matchExpressions": [{"key": "track", "operator": "Near", "values": ["canary"]}]});
        k.spec.as_mut().unwrap().selector = serde_json::from_value(unknown).unwrap();
        assert_eq!(deployment(&k).unwrap().selector, "", "can't be read");
    }

    #[test]
    fn a_finished_init_container_is_no_termination() {
        let mut raw: core::Pod = read("pod-running");
        let done = serde_json::json!({
            "name": "wait-for-db", "ready": true, "restartCount": 0, "image": "busybox", "imageID": "",
            "state": {"terminated": {"exitCode": 0, "reason": "Completed", "finishedAt": "2026-11-01T10:00:00Z"}}
        });
        raw.status.as_mut().unwrap().init_container_statuses = Some(vec![serde_json::from_value(done).unwrap()]);
        let p = pod(&raw).unwrap();
        assert_eq!((p.containers[0].init, p.containers[0].state.as_str()), (true, "terminated: Completed (exit 0)"));
        assert_eq!(p.containers[0].last_termination, None);
        assert_eq!(p.last_termination, None, "a healthy pod shows no termination");
    }

    #[test]
    fn a_running_pod() {
        let p = pod_fixture("running");
        assert_eq!(p.name, "graphservice-6d9c4b8f7-x2k4p");
        assert_eq!((p.owner.as_deref(), p.template.as_str()), (Some("graphservice"), "6d9c4b8f7"));
        assert_eq!((p.phase, p.ready, p.restarts, p.last_termination), (PodPhase::Running, true, 0, None));
        assert_eq!(p.started, Some(epoch("2026-11-07T16:20:41Z")));
        assert_eq!((p.node.as_str(), p.tag.as_str()), ("gke-km011gcp-pool-2-8f3a1c2d-x7kq", "lan-000-slim"));
        assert_eq!(p.containers, [ContainerStatus { name: "graphservice".into(), init: false, ready: true, restarts: 0, state: "running".into(), message: None, last_termination: None }]);
    }

    #[test]
    fn a_crash_looping_pod_says_how_it_last_ended() {
        let p = pod_fixture("crash-loop");
        assert_eq!((p.phase, p.ready, p.restarts), (PodPhase::Waiting("CrashLoopBackOff".into()), false, 14));
        let ended = Termination { reason: "Error".into(), exit_code: Some(2), message: Some("panic: missing TADA_DB_URL".into()), at: Some(epoch("2026-11-08T08:57:12Z")) };
        assert_eq!(p.last_termination.as_ref(), Some(&ended), "the message's first line");
        assert_eq!(p.containers[0].state, "waiting: CrashLoopBackOff");
        assert_eq!(p.containers[0].last_termination, Some(ended));
        assert_eq!(p.tag, "lan-160");
    }

    #[test]
    fn an_image_that_cant_be_pulled() {
        let p = pod_fixture("image-pull");
        assert_eq!((p.phase, p.ready, p.restarts, p.last_termination), (PodPhase::Waiting("ImagePullBackOff".into()), false, 0, None));
        assert_eq!((p.containers[0].state.as_str(), p.tag.as_str()), ("waiting: ImagePullBackOff", "lan-999"));
    }

    #[test]
    fn a_crash_looping_init_container_comes_first() {
        let p = pod_fixture("init-crash-loop");
        assert_eq!((p.phase, p.ready, p.restarts), (PodPhase::Waiting("Init:CrashLoopBackOff".into()), false, 6), "not the main container's PodInitializing");
        let states: Vec<_> = p.containers.iter().map(|c| (c.name.as_str(), c.init, c.state.as_str())).collect();
        assert_eq!(states, [("wait-for-db", true, "waiting: CrashLoopBackOff"), ("notifier", false, "waiting: PodInitializing")]);
        assert_eq!(p.last_termination.map(|t| (t.reason, t.exit_code, t.message)), Some(("Error".into(), Some(1), None)));
        assert_eq!(p.tag, "1.4.0", "the main container's, not the init container's");
    }

    #[test]
    fn a_pod_being_deleted_is_terminating() {
        let p = pod_fixture("terminating");
        assert_eq!((p.phase, p.ready, p.owner.as_deref()), (PodPhase::Terminating, true, Some("graphservice")));
    }

    #[test]
    fn an_evicted_pod_failed_with_its_reason() {
        let p = pod_fixture("evicted");
        assert_eq!((p.phase, p.ready), (PodPhase::Failed("Evicted".into()), false));
        assert_eq!(p.containers[0].state, "terminated: OOMKilled (exit 137)");
        assert_eq!(p.last_termination.map(|t| (t.reason, t.exit_code, t.at)), Some(("OOMKilled".into(), Some(137), Some(epoch("2026-11-07T03:40:10Z")))), "its current state's");
        let mut k: core::Pod = read("pod-evicted");
        k.status.as_mut().unwrap().reason = None;
        assert_eq!(pod(&k).unwrap().phase, PodPhase::Failed("Failed".into()));
        k.status.as_mut().unwrap().phase = Some("Succeeded".into());
        assert_eq!(pod(&k).unwrap().phase, PodPhase::Succeeded);
    }

    #[test]
    fn a_pending_pod_has_no_start_node_or_containers() {
        let p = pod_fixture("pending");
        assert_eq!((p.phase, p.ready, p.started, p.node.as_str()), (PodPhase::Pending, false, None, ""));
        assert!(p.containers.is_empty() && p.last_termination.is_none());
        assert_eq!(p.tag, "2.0.1", "from the spec");
    }

    #[test]
    fn two_containers_add_up_and_the_main_one_names_the_tag() {
        let p = pod_fixture("two-containers");
        assert_eq!((p.phase, p.ready, p.restarts), (PodPhase::Running, true, 3));
        assert_eq!(p.tag, "lan-160", "the container named like the deployment, though not the first");
        assert_eq!(p.last_termination.map(|t| (t.reason, t.at)), Some(("Completed".into(), Some(epoch("2026-11-07T21:15:08Z")))), "the latest of the two");
        let restarts: Vec<_> = p.containers.iter().map(|c| (c.name.as_str(), c.restarts)).collect();
        assert_eq!(restarts, [("entityservice", 1), ("envoy", 2)]);
    }

    #[test]
    fn owners_other_than_a_replica_set_or_none_give_no_owner() {
        let mut k: core::Pod = read("pod-running");
        k.metadata.owner_references.as_mut().unwrap()[0].kind = "StatefulSet".into();
        let p = pod(&k).unwrap();
        assert_eq!((p.owner, p.tag.as_str()), (None, "lan-000-slim"), "the first container without an owner");
        k.metadata.owner_references = None;
        k.metadata.labels = None;
        assert_eq!((pod(&k).unwrap().owner, pod(&k).unwrap().template), (None, String::new()));
        k.metadata.name = None;
        assert_eq!(pod(&k), None);
    }
}
