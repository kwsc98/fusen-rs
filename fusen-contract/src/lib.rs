#![warn(missing_docs)]
//! Stable HTTP binding and service registry contracts for fusen-rs.

mod http;
mod sensitivity;
mod service;

pub use http::{
    EndpointCapabilities, HTTP_JSON_V1, HttpBindingId, HttpVersionPolicy, HttpVersionSet,
};
pub use sensitivity::{
    MethodSensitivity, SensitiveArgument, SensitiveField, SensitiveFields, SensitiveShape,
    SensitiveShapeResolver, SensitivityKind,
};
pub use service::{
    ContractError, HttpOperation, HttpParameter, HttpParameterCardinality, HttpParameterSource,
    InstanceId, Metadata, MethodDescriptor, MethodId, ServiceDescriptor, ServiceEndpoint,
    ServiceInstance, ServiceRegistration, ServiceSelector, ServiceWeight,
};

#[cfg(feature = "derive")]
pub use fusen_procedural_macro::SensitiveFields;
