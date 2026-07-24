#![warn(missing_docs)]
//! Stable protocol and service registry contracts for fusen-rs.

mod protocol;
mod service;

pub use protocol::WireProtocol;
pub use service::{
    ContractError, Metadata, MethodDescriptor, MethodId, ParameterDescriptor, ParameterSource,
    ServiceDescriptor, ServiceEndpoint, ServiceInstance, ServiceRegistration, ServiceSelector,
    ServiceWeight,
};

/// Borrowing, sendable future used by object-safe asynchronous contracts.
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

/// Owned, sendable future used by asynchronous contracts that do not borrow their caller.
pub type StaticBoxFuture<T> = BoxFuture<'static, T>;
