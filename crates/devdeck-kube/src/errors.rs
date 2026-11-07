//! kube errors as messages a user can act on (R7.8).

use crate::Target;
use kube::runtime::watcher;
use std::error::Error;

/// What went wrong reading `target`, in words a user can act on.
pub fn describe(e: &kube::Error, target: &Target) -> String {
    let (cluster, env) = (&target.cluster, &target.env);
    if let kube::Error::Api(status) = e {
        return match status.code {
            401 => format!("{cluster} refused your credentials"),
            403 => format!("your account can't list deployments in {env}"),
            code => {
                let message = if status.message.is_empty() { &status.reason } else { &status.message };
                format!("{cluster} ({env}): {message} ({code})")
            }
        };
    }
    match unreachable(e) {
        Some(cause) => format!("can't reach {cluster} ({env}): {cause}"),
        None => e.to_string(),
    }
}

/// What a watcher's error says, as [`describe`].
pub(crate) fn describe_watcher(e: &watcher::Error, target: &Target) -> String {
    match e {
        watcher::Error::InitialListFailed(e) | watcher::Error::WatchStartFailed(e) | watcher::Error::WatchFailed(e) => describe(e, target),
        watcher::Error::WatchError(status) => describe(&kube::Error::Api(status.clone()), target),
        watcher::Error::NoResourceVersion => e.to_string(),
    }
}

/// The root cause when the cluster couldn't be reached: a connection refused or reset, a
/// timeout, a name that doesn't resolve (an I/O error somewhere in the chain), or a
/// connection hyper gave up on.
fn unreachable(e: &kube::Error) -> Option<String> {
    let chain: Vec<&(dyn Error + 'static)> = std::iter::successors(Some(e as &(dyn Error + 'static)), |&e| e.source()).collect();
    let io = chain.iter().find_map(|e| e.downcast_ref::<std::io::Error>());
    if io.is_none() && !matches!(e, kube::Error::HyperError(_)) {
        return None;
    }
    if io.is_some_and(|io| io.kind() == std::io::ErrorKind::TimedOut) {
        return Some("connection timed out".into());
    }
    chain.last().map(|root| root.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::core::Status;
    use std::io;

    fn target() -> Target {
        Target::for_tests("dev35", "km011gcp")
    }

    fn api(code: u16, message: &str, reason: &str) -> kube::Error {
        kube::Error::Api(Status::failure(message, reason).with_code(code).boxed())
    }

    /// An error wrapping another, as hyper-util's connector errors do.
    #[derive(Debug)]
    struct Wrapped(&'static str, Box<dyn Error + Send + Sync>);

    impl std::fmt::Display for Wrapped {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl Error for Wrapped {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&*self.1)
        }
    }

    fn service(cause: impl Error + Send + Sync + 'static) -> kube::Error {
        kube::Error::Service(Box::new(Wrapped("client error (Connect)", Box::new(cause))))
    }

    #[test]
    fn credentials_and_permissions() {
        assert_eq!(describe(&api(401, "Unauthorized", "Unauthorized"), &target()), "km011gcp refused your credentials");
        let forbidden = "deployments.apps is forbidden: User \"system:serviceaccount:x\" cannot list resource \"deployments\"";
        assert_eq!(describe(&api(403, forbidden, "Forbidden"), &target()), "your account can't list deployments in dev35");
    }

    #[test]
    fn other_api_errors_keep_their_message_and_code() {
        assert_eq!(describe(&api(404, "the server could not find the requested resource", "NotFound"), &target()), "km011gcp (dev35): the server could not find the requested resource (404)");
        assert_eq!(describe(&api(503, "", "ServiceUnavailable"), &target()), "km011gcp (dev35): ServiceUnavailable (503)");
    }

    #[test]
    fn unreachable_clusters_give_the_root_cause() {
        let refused = service(Wrapped("tcp connect error", Box::new(io::Error::new(io::ErrorKind::ConnectionRefused, "Connection refused (os error 111)"))));
        assert_eq!(describe(&refused, &target()), "can't reach km011gcp (dev35): Connection refused (os error 111)");
        let dns = service(Wrapped("dns error", Box::new(io::Error::other("failed to lookup address information: Name or service not known"))));
        assert_eq!(describe(&dns, &target()), "can't reach km011gcp (dev35): failed to lookup address information: Name or service not known");
        let timeout = service(io::Error::new(io::ErrorKind::TimedOut, "deadline has elapsed"));
        assert_eq!(describe(&timeout, &target()), "can't reach km011gcp (dev35): connection timed out");
        let reset = kube::Error::ReadEvents(io::Error::new(io::ErrorKind::ConnectionReset, "connection reset by peer"));
        assert_eq!(describe(&reset, &target()), "can't reach km011gcp (dev35): connection reset by peer");
    }

    #[test]
    fn anything_else_is_its_text() {
        let e = kube::Error::LinesCodecMaxLineLengthExceeded;
        assert_eq!(describe(&e, &target()), e.to_string());
        let watch = watcher::Error::WatchError(Status::failure("too old resource version", "Expired").with_code(410).boxed());
        assert_eq!(describe_watcher(&watch, &target()), "km011gcp (dev35): too old resource version (410)");
    }
}
