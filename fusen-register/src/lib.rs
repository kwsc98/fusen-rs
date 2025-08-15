use fusen_internal_common::{BoxFuture, resource::service::ServiceResource};
use std::sync::Arc;

use crate::{directory::Directory, error::RegisterError};

pub mod directory;
pub mod error;
pub mod support;

pub trait Register: Send + Sync {
    fn register(&self, resource: Arc<ServiceResource>) -> BoxFuture<Result<(), RegisterError>>;

    fn deregister(&self, resource: Arc<ServiceResource>) -> BoxFuture<Result<(), RegisterError>>;

    fn subscribe(&self, resource: ServiceResource) -> BoxFuture<Result<Directory, RegisterError>>;
}
