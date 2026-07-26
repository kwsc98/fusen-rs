#![warn(missing_docs)]
//! Stable protocol and service registry contracts for fusen-rs.

mod protocol;
mod service;

pub use protocol::{Idempotency, ProtocolSet, WireProtocol};
pub use service::{
    ContractError, InstanceId, Metadata, MethodDescriptor, MethodId, ServiceDescriptor,
    ServiceEndpoint, ServiceInstance, ServiceRegistration, ServiceSelector, ServiceWeight,
    SpringCloudMethod, SpringCloudParameter, SpringCloudParameterSource,
};
