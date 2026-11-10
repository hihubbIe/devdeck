//! Cluster writes (R7.3): a deployment's image changed, and whether the account may, on
//! the watches' runtime and shared clients.
//!
//! - The image is changed by a JSON Patch that first tests the container at that index is
//!   still the one confirmed (its name and image), so what runs is what the user saw; the
//!   API server answers a failed test with a 422 whose message is the generic "the server
//!   rejected our request due to an error in our request" (apiserver's patch handler drops
//!   json-patch's own words), read as [`DeployError::Changed`].
//! - A 403 is the account's only when it is an RBAC refusal (`… is forbidden: User "…"
//!   cannot patch …`); an admission webhook answers 403 too, in its own words.
//! - Each call is bounded in time: a connection that died without a word answers too.

use crate::Target;
use crate::driver::{client, runtime, until_closed};
use crate::errors::describe;
use k8s_openapi::api::apps::v1 as apps;
use k8s_openapi::api::authorization::v1::{ResourceAttributes, SelfSubjectAccessReview, SelfSubjectAccessReviewSpec};
use k8s_openapi::serde_json::{self, json};
use kube::api::{Patch, PatchParams, PostParams};
use kube::{Api, Client};
use std::time::Duration;

/// How long an image change may take, connecting included.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long an access review may take.
const CAN_TIMEOUT: Duration = Duration::from_secs(10);
/// The field manager devdeck's writes carry (the deployment's `managedFields` say who
/// changed it).
pub const FIELD_MANAGER: &str = "devdeck";
/// What an image change does, for messages.
const DOING: &str = "change deployments";
/// What an access review does, for messages.
const CHECKING: &str = "check what your account may do";
/// What the API server says when a JSON Patch doesn't apply (a `test` that failed, a path
/// gone).
const PATCH_REJECTED: &str = "the server rejected our request due to an error in our request";

/// One image change, as confirmed: the container (its index in the pod template and its
/// name), the image it runs now, the new one.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageChange {
    pub index: usize,
    pub container: String,
    pub from: String,
    pub to: String,
}

/// Why an image change was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum DeployError {
    /// An RBAC refusal: "your account can't change deployments in <env>".
    Forbidden(String),
    /// 404: "the deployment <name> is gone from <env>".
    Gone(String),
    /// The image isn't the one confirmed any more (the JSON Patch test failed): "<name>
    /// changed since you confirmed".
    Changed(String),
    /// Anything else, in the cluster's words (an admission webhook's 403 included),
    /// described like the watches' errors.
    Other(String),
}

impl std::fmt::Display for DeployError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployError::Forbidden(m) | DeployError::Gone(m) | DeployError::Changed(m) | DeployError::Other(m) => f.write_str(m),
        }
    }
}

/// Sets the image: a JSON Patch that tests `/spec/template/spec/containers/<index>/image`
/// equals `from` and the container at that index is named `container`, then replaces the
/// image with `to`; sent with field manager `devdeck`. Answers once with the generation
/// the cluster gave the changed spec.
///
/// The write goes on when the receiver is dropped: a request sent can't be taken back, so
/// it is better finished than cut at an unknown point.
pub fn set_image(target: &Target, deployment: &str, change: &ImageChange) -> async_channel::Receiver<Result<u64, DeployError>> {
    let (tx, rx) = async_channel::bounded(1);
    let (target, deployment, change) = (target.clone(), deployment.to_owned(), change.clone());
    runtime().spawn(async move {
        let write = async {
            match client(&target).await {
                Ok((client, _user)) => patch_image(client, &target, &deployment, &change).await,
                Err(e) => Err(DeployError::Other(describe(&e, &target, DOING))),
            }
        };
        let _ = tx.try_send(within_write(&target, write).await);
    });
    rx
}

/// `write`, or "didn't answer" after [`WRITE_TIMEOUT`].
async fn within_write(target: &Target, write: impl Future<Output = Result<u64, DeployError>>) -> Result<u64, DeployError> {
    tokio::time::timeout(WRITE_TIMEOUT, write).await.unwrap_or_else(|_| Err(DeployError::Other(format!("{} ({}) didn't answer the image change: it may have gone through, the rollout will say", target.cluster, target.env))))
}

/// The JSON Patch of `change`.
fn image_patch(change: &ImageChange) -> serde_json::Value {
    let container = format!("/spec/template/spec/containers/{}", change.index);
    json!([
        {"op": "test", "path": format!("{container}/name"), "value": change.container},
        {"op": "test", "path": format!("{container}/image"), "value": change.from},
        {"op": "replace", "path": format!("{container}/image"), "value": change.to},
    ])
}

/// Sends `change` to `deployment` of `target` through `client`.
async fn patch_image(client: Client, target: &Target, deployment: &str, change: &ImageChange) -> Result<u64, DeployError> {
    // kube's `Patch::Json` holds a `json_patch::Patch`, read from the JSON it is.
    let operations = serde_json::from_value(image_patch(change)).map_err(|e| DeployError::Other(e.to_string()))?;
    let params = PatchParams { field_manager: Some(FIELD_MANAGER.into()), ..Default::default() };
    let api: Api<apps::Deployment> = Api::namespaced(client, &target.namespace);
    match api.patch(deployment, &params, &Patch::Json::<()>(operations)).await {
        Ok(changed) => Ok(changed.metadata.generation.unwrap_or(0).max(0) as u64),
        Err(e) => Err(refusal(&e, target, deployment)),
    }
}

/// What the cluster's refusal of an image change means.
fn refusal(e: &kube::Error, target: &Target, deployment: &str) -> DeployError {
    let kube::Error::Api(status) = e else {
        return DeployError::Other(describe(e, target, DOING));
    };
    let message = if status.message.is_empty() { &status.reason } else { &status.message };
    match status.code {
        403 if rbac(&status.message) => DeployError::Forbidden(format!("your account can't {DOING} in {}", target.env)),
        403 => DeployError::Other(format!("{} ({}): {message} (403)", target.cluster, target.env)),
        404 => DeployError::Gone(format!("the deployment {deployment} is gone from {}", target.env)),
        422 | 409 if test_failed(status) => DeployError::Changed(format!("{deployment} changed since you confirmed")),
        _ => DeployError::Other(describe(e, target, DOING)),
    }
}

/// Whether a 403's message is RBAC's (`deployments.apps "x" is forbidden: User "…" cannot
/// patch resource "deployments" …`), not an admission webhook's or a policy's.
fn rbac(message: &str) -> bool {
    message.contains(" is forbidden: User \"") && message.contains("\" cannot ")
}

/// Whether a refusal says the patch's `test` failed: apiserver's generic words for a JSON
/// Patch that doesn't apply, or json-patch's own when a server passes them on.
fn test_failed(status: &kube::core::Status) -> bool {
    let causes = status.details.iter().flat_map(|d| &d.causes).map(|c| c.message.as_str());
    std::iter::once(status.message.as_str()).chain(causes).any(|m| {
        let m = m.to_lowercase();
        m.contains(PATCH_REJECTED) || m.contains("test failed") || m.contains("testing value") || m.contains("test operation")
    })
}

/// Whether the account may `verb` `resource` of `group` in the target's namespace: one
/// SelfSubjectAccessReview (authorization.k8s.io/v1), what `kubectl auth can-i` asks.
/// Answers once.
pub fn can(target: &Target, verb: &str, group: &str, resource: &str) -> async_channel::Receiver<Result<bool, String>> {
    let (tx, rx) = async_channel::bounded(1);
    let (target, verb, group, resource) = (target.clone(), verb.to_owned(), group.to_owned(), resource.to_owned());
    let sender = tx.clone();
    runtime().spawn(until_closed(tx, async move {
        let review = async {
            match client(&target).await {
                Ok((client, _user)) => review(client, &target, &verb, &group, &resource).await,
                Err(e) => Err(describe(&e, &target, CHECKING)),
            }
        };
        let _ = sender.try_send(within_review(&target, review).await);
    }));
    rx
}

/// `review`, or "didn't answer" after [`CAN_TIMEOUT`].
async fn within_review(target: &Target, review: impl Future<Output = Result<bool, String>>) -> Result<bool, String> {
    tokio::time::timeout(CAN_TIMEOUT, review).await.unwrap_or_else(|_| Err(format!("{} ({}) didn't answer the access review", target.cluster, target.env)))
}

/// One access review through `client`.
async fn review(client: Client, target: &Target, verb: &str, group: &str, resource: &str) -> Result<bool, String> {
    let attributes = ResourceAttributes { namespace: Some(target.namespace.clone()), verb: Some(verb.into()), group: Some(group.into()), resource: Some(resource.into()), ..Default::default() };
    let review = SelfSubjectAccessReview { spec: SelfSubjectAccessReviewSpec { resource_attributes: Some(attributes), ..Default::default() }, ..Default::default() };
    let api: Api<SelfSubjectAccessReview> = Api::all(client);
    let answer = api.create(&PostParams::default(), &review).await.map_err(|e| describe(&e, target, CHECKING))?;
    answer.status.map(|s| s.allowed).ok_or_else(|| format!("{} ({}) gave no answer to the access review", target.cluster, target.env))
}

#[cfg(test)]
mod tests;
