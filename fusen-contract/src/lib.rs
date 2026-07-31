#![warn(missing_docs)]
//! Stable protocol and service registry contracts for fusen-rs.

mod protocol;
mod sensitivity;
mod service;

pub use protocol::{ProtocolSet, WireProtocol};
pub use sensitivity::{
    MethodSensitivity, SensitiveArgument, SensitiveField, SensitiveFields, SensitiveShape,
    SensitiveShapeResolver, SensitivityKind,
};
pub use service::{
    ContractError, InstanceId, Metadata, MethodDescriptor, MethodId, ServiceDescriptor,
    ServiceEndpoint, ServiceInstance, ServiceRegistration, ServiceSelector, ServiceWeight,
    SpringCloudMethod, SpringCloudParameter, SpringCloudParameterCardinality,
    SpringCloudParameterSource,
};

#[cfg(feature = "derive")]
pub use fusen_procedural_macro::SensitiveFields;
