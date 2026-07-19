#![warn(missing_docs)]
//! Service registration and discovery contracts for fusen-rs.

use fusen_internal_common::{
    BoxFuture, protocol::WireProtocol, resource::service::ServiceResource,
};
use std::sync::Arc;

use crate::{directory::Directory, error::RegisterError};

/// Atomic service instance snapshots.
pub mod directory;
/// Registration and directory failures.
pub mod error;
/// Shared types used when implementing [`Register`].
pub use fusen_internal_common;

/// Pluggable service registry used by clients and servers.
pub trait Register: Send + Sync {
    /// Publishes one service instance.
    fn register(
        &self,
        resource: Arc<ServiceResource>,
        protocol: WireProtocol,
    ) -> BoxFuture<Result<(), RegisterError>>;

    /// Removes one previously published service instance.
    fn deregister(
        &self,
        resource: Arc<ServiceResource>,
        protocol: WireProtocol,
    ) -> BoxFuture<Result<(), RegisterError>>;

    /// Subscribes to all instances matching the requested service resource.
    fn subscribe(
        &self,
        resource: ServiceResource,
        protocol: WireProtocol,
    ) -> BoxFuture<Result<Directory, RegisterError>>;
}
