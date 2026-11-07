//! Cluster access for devdeck (R7), without UI: the kubeconfigs of a workspace's folder,
//! an env's target (file, context, namespace) and a live watch of its deployments, fed
//! from one tokio runtime started on the first watch. The app's `Cluster` entity reads
//! what a watch sends over its channel.

pub mod convert;
pub mod driver;
pub mod errors;
pub mod kubeconfigs;

pub use driver::{EnvUpdate, WatchHandle, watch};
pub use kubeconfigs::{Kubeconfigs, Target};
