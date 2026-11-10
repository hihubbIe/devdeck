//! Image changes and access reviews against a mock API: each request recorded whole, each
//! answered as the API words it, on paused time.

use super::*;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use serde_json::Value;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Reply {
    /// This code and JSON body.
    Json(u16, Value),
    /// Never answers.
    Never,
}

/// A request as the mock received it.
#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    query: String,
    content_type: String,
    body: Value,
}

/// A client to a mock answering every request with `reply`, and the requests it received.
fn mock(reply: Reply) -> (Client, Arc<Mutex<Vec<Request>>>) {
    let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
    let received = requests.clone();
    let service = tower::service_fn(move |req: http::Request<kube::client::Body>| {
        let (received, reply) = (received.clone(), reply.clone());
        async move {
            let (parts, body) = req.into_parts();
            let bytes = body.collect().await.unwrap().to_bytes();
            received.lock().unwrap().push(Request {
                method: parts.method.to_string(),
                path: parts.uri.path().into(),
                query: parts.uri.query().unwrap_or_default().into(),
                content_type: parts.headers.get(http::header::CONTENT_TYPE).map(|v| v.to_str().unwrap().to_owned()).unwrap_or_default(),
                body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            });
            let (code, body) = match reply {
                Reply::Json(code, body) => (code, body),
                Reply::Never => std::future::pending().await,
            };
            let mut response = http::Response::new(Full::new(Bytes::from(body.to_string())));
            *response.status_mut() = http::StatusCode::from_u16(code).unwrap();
            Ok::<_, Infallible>(response)
        }
    });
    (Client::new(service, "dev35"), requests)
}

fn target() -> Target {
    Target::for_tests("dev35", "km011gcp")
}

fn status(code: u16, reason: &str, message: &str) -> Reply {
    Reply::Json(code, json!({"apiVersion": "v1", "kind": "Status", "metadata": {}, "status": "Failure", "message": message, "reason": reason, "code": code}))
}

fn change() -> ImageChange {
    ImageChange { index: 1, container: "api".into(), from: "registry/api:main".into(), to: "registry/api:feature-y".into() }
}

/// The deployment the API answers a patch with: the changed spec, generation 8.
fn changed() -> Reply {
    Reply::Json(
        200,
        json!({
            "apiVersion": "apps/v1", "kind": "Deployment",
            "metadata": {"name": "api", "namespace": "dev35", "generation": 8, "resourceVersion": "9182"},
            "spec": {
                "selector": {"matchLabels": {"app": "api"}},
                "template": {"metadata": {"labels": {"app": "api"}}, "spec": {"containers": [{"name": "envoy", "image": "envoy:1.30"}, {"name": "api", "image": "registry/api:feature-y"}]}}
            },
            "status": {"observedGeneration": 7}
        }),
    )
}

/// The image change of [`change`] on `api` of dev35, as [`set_image`] runs it, against a
/// mock answering `reply`.
async fn set(reply: Reply) -> (Result<u64, DeployError>, Vec<Request>) {
    let (client, requests) = mock(reply);
    let result = within_write(&target(), patch_image(client, &target(), "api", &change())).await;
    let requests = requests.lock().unwrap().clone();
    (result, requests)
}

async fn may(reply: Reply) -> (Result<bool, String>, Vec<Request>) {
    let (client, requests) = mock(reply);
    let result = within_review(&target(), review(client, &target(), "patch", "apps", "deployments")).await;
    let requests = requests.lock().unwrap().clone();
    (result, requests)
}

#[tokio::test(start_paused = true)]
async fn the_patch_tests_the_confirmed_container_then_replaces_its_image() {
    let (result, requests) = set(changed()).await;
    assert_eq!(result, Ok(8), "the generation of the changed spec");
    let [request] = requests.as_slice() else { panic!("one request: {requests:?}") };
    assert_eq!((request.method.as_str(), request.path.as_str()), ("PATCH", "/apis/apps/v1/namespaces/dev35/deployments/api"));
    let query: Vec<&str> = request.query.split('&').filter(|p| !p.is_empty()).collect();
    assert_eq!(query, ["fieldManager=devdeck"], "kube-core writes it `&fieldManager=…`");
    assert_eq!(request.content_type, "application/json-patch+json");
    let body = json!([
        {"op": "test", "path": "/spec/template/spec/containers/1/name", "value": "api"},
        {"op": "test", "path": "/spec/template/spec/containers/1/image", "value": "registry/api:main"},
        {"op": "replace", "path": "/spec/template/spec/containers/1/image", "value": "registry/api:feature-y"},
    ]);
    assert_eq!(request.body, body);
}

#[tokio::test(start_paused = true)]
async fn an_rbac_refusal_is_the_accounts() {
    let message = "deployments.apps \"api\" is forbidden: User \"jane@example.com\" cannot patch resource \"deployments\" in API group \"apps\" in the namespace \"dev35\"";
    let (result, _) = set(status(403, "Forbidden", message)).await;
    assert_eq!(result, Err(DeployError::Forbidden("your account can't change deployments in dev35".into())));
    let service_account = "deployments.apps \"api\" is forbidden: User \"system:serviceaccount:ci:deployer\" cannot patch resource \"deployments\" in API group \"apps\" in the namespace \"dev35\"";
    let (result, _) = set(status(403, "Forbidden", service_account)).await;
    assert!(matches!(result, Err(DeployError::Forbidden(_))), "{result:?}");
}

#[tokio::test(start_paused = true)]
async fn a_webhooks_403_is_in_its_own_words() {
    let message = "admission webhook \"validate.kyverno.svc-fail\" denied the request: images must come from the approved registry";
    let (result, _) = set(status(403, "Forbidden", message)).await;
    assert_eq!(result, Err(DeployError::Other(format!("km011gcp (dev35): {message} (403)"))));
    let quota = "pods \"api-1\" is forbidden: exceeded quota: compute";
    let (result, _) = set(status(403, "Forbidden", quota)).await;
    assert_eq!(result, Err(DeployError::Other(format!("km011gcp (dev35): {quota} (403)"))), "a policy, not the account");
}

#[tokio::test(start_paused = true)]
async fn a_deployment_gone_says_so() {
    let (result, _) = set(status(404, "NotFound", "deployments.apps \"api\" not found")).await;
    assert_eq!(result, Err(DeployError::Gone("the deployment api is gone from dev35".into())));
}

#[tokio::test(start_paused = true)]
async fn a_failed_test_means_it_changed_since_confirmed() {
    // What apiserver's patch handler answers when a JSON Patch doesn't apply.
    let (result, _) = set(status(422, "Invalid", "the server rejected our request due to an error in our request")).await;
    assert_eq!(result, Err(DeployError::Changed("api changed since you confirmed".into())));
    // json-patch's own words, when a server passes them on.
    let cause = Reply::Json(
        422,
        json!({"apiVersion": "v1", "kind": "Status", "metadata": {}, "status": "Failure", "message": "invalid patch", "reason": "Invalid", "code": 422,
               "details": {"causes": [{"message": "testing value /spec/template/spec/containers/1/image failed: test failed"}]}}),
    );
    assert_eq!(set(cause).await.0, Err(DeployError::Changed("api changed since you confirmed".into())));
}

#[tokio::test(start_paused = true)]
async fn other_refusals_keep_the_clusters_words() {
    let invalid = "Deployment.apps \"api\" is invalid: spec.template.spec.containers[1].image: Invalid value: \"registry/api:Feature Y\"";
    let (result, _) = set(status(422, "Invalid", invalid)).await;
    assert_eq!(result, Err(DeployError::Other(format!("km011gcp (dev35): {invalid} (422)"))), "validation isn't a failed test");
    let (result, _) = set(status(409, "Conflict", "Operation cannot be fulfilled on deployments.apps \"api\": the object has been modified")).await;
    assert!(matches!(result, Err(DeployError::Other(_))), "{result:?}");
    let (result, _) = set(status(401, "Unauthorized", "Unauthorized")).await;
    assert_eq!(result, Err(DeployError::Other("km011gcp refused your credentials".into())));
}

#[tokio::test(start_paused = true)]
async fn a_patch_without_an_answer_times_out() {
    let (result, requests) = set(Reply::Never).await;
    assert_eq!(result, Err(DeployError::Other("km011gcp (dev35) didn't answer the image change: it may have gone through, the rollout will say".into())));
    assert_eq!(requests.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_access_review_asks_for_the_namespace_verb_group_and_resource() {
    let answer = |allowed: bool| {
        Reply::Json(
            201,
            json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview", "metadata": {},
                   "spec": {"resourceAttributes": {"namespace": "dev35", "verb": "patch", "group": "apps", "resource": "deployments"}},
                   "status": {"allowed": allowed}}),
        )
    };
    let (result, requests) = may(answer(true)).await;
    assert_eq!(result, Ok(true));
    let [request] = requests.as_slice() else { panic!("one request: {requests:?}") };
    assert_eq!((request.method.as_str(), request.path.as_str()), ("POST", "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews"));
    let body = json!({
        "apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview", "metadata": {},
        "spec": {"resourceAttributes": {"namespace": "dev35", "verb": "patch", "group": "apps", "resource": "deployments"}}
    });
    assert_eq!(request.body, body);
    assert_eq!(may(answer(false)).await.0, Ok(false));
}

#[tokio::test(start_paused = true)]
async fn an_access_review_that_fails_says_why() {
    let (result, _) = may(status(403, "Forbidden", "selfsubjectaccessreviews.authorization.k8s.io is forbidden")).await;
    assert_eq!(result, Err("your account can't check what your account may do in dev35".into()));
    let (result, _) = may(status(500, "InternalError", "etcd timed out")).await;
    assert_eq!(result, Err("km011gcp (dev35): etcd timed out (500)".into()));
    let (result, _) = may(Reply::Json(201, json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview", "metadata": {}, "spec": {}}))).await;
    assert_eq!(result, Err("km011gcp (dev35) gave no answer to the access review".into()));
    let (result, _) = may(Reply::Never).await;
    assert_eq!(result, Err("km011gcp (dev35) didn't answer the access review".into()));
}
