use fusen_internal_common::{BoxFuture, protocol::Protocol, resource::service::ServiceResource};
use std::sync::Arc;

use crate::{directory::Directory, error::RegisterError};

pub mod directory;
pub mod error;
pub use fusen_internal_common;

pub trait Register: Send + Sync {
    fn register(
        &self,
        resource: Arc<ServiceResource>,
        protocol: Protocol,
    ) -> BoxFuture<Result<(), RegisterError>>;

    fn deregister(
        &self,
        resource: Arc<ServiceResource>,
        protocol: Protocol,
    ) -> BoxFuture<Result<(), RegisterError>>;

    fn subscribe(
        &self,
        resource: ServiceResource,
        protocol: Protocol,
    ) -> BoxFuture<Result<Directory, RegisterError>>;
}
