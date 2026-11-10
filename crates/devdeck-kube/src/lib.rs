//! Cluster access for devdeck (R7), without UI: the kubeconfigs of a workspace's folder,
//! an env's target (file, context, namespace), live watches of its deployments and pods,
//! reads of its pods' metrics and streams of their logs, run on one tokio runtime started
//! on first use. The app's `Cluster` entity reads what they send over their channels.

pub mod convert;
pub mod driver;
pub mod errors;
pub mod kubeconfigs;
pub mod logs;

pub use driver::{EnvUpdate, MetricsError, PodUpdate, WatchHandle, metrics, watch, watch_pods};
pub use kubeconfigs::{Kubeconfigs, Target};
pub use logs::{LogFrom, LogLine, LogRefusal, LogUpdate, logs};
