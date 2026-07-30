#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Production-oriented JSON RPC runtime with HTTP/HTTPS clients and a plaintext HTTP/1.1/h2c server.

extern crate self as fusen_rs;

mod client;
mod context;
mod error;
/// Interface DTO schema and message encoding contracts.
pub mod interface;
/// Shared client/server middleware API.
pub mod middleware;
/// Client routing, load balancing, and retry policy APIs.
pub mod policy;
mod resilience;
mod rpc;
mod runtime;
mod server;
mod service;
mod wire;

pub use client::{
    BreakerThreshold, BreakerThresholdBuilder, CircuitBreakerConfig, CircuitBreakerConfigBuilder,
    ClientAdmissionConfig, ClientAdmissionConfigBuilder, ClientBuilder, ClientConfig,
    ClientConfigBuilder, ClientHttpConfig, ClientHttpConfigBuilder, ClientRuntime,
    ClientRuntimeBuilder, ClientState, DiscoveryConfig, DiscoveryConfigBuilder, QueueConfig,
    QueueConfigBuilder, RetryConfig, RetryConfigBuilder,
};
pub use context::{
    CallInfo, MiddlewareStage, RpcArguments, RpcBody, RpcContext, RpcRequest, RpcResponse, RpcSide,
};
pub use error::{
    ClientError, ClientErrorKind, ConfigValidationError, ConfigValidationErrorKind, ServerError,
    ServerErrorKind,
};
pub use fusen_contract as contract;
pub use fusen_contract::{Idempotency, WireProtocol};
pub use fusen_observability::{MetricsRecorder, NoopMetricsRecorder};
pub use fusen_procedural_macro::{RpcMessage, interface, method};
pub use fusen_register::{RegistrationHandle, Registry, SubscriptionHandle};
pub use interface::{RpcField, RpcFieldSource, RpcMessage};
pub use middleware::{Middleware, MiddlewareFuture, MiddlewareResult, Next};
pub use policy::{InstanceRouter, InstanceSnapshot, LoadBalancer, RouteRequest, WeightedRandom};
pub use resilience::{FailureClass, RetryDecision, RetryDecisionContext, RetryPolicy};
pub use rpc::{
    ErrorCode, InvalidErrorCode, RetryHint, RpcCategory, RpcError, RpcErrorDetails, RpcOrigin,
};
pub use server::{
    HttpServerConfig, HttpServerConfigBuilder, RunningServer, Server, ServerBuilder, ServerConfig,
    ServerConfigBuilder, ServerHandle, ServerRegistryConfig, ServerRegistryConfigBuilder,
    ServerRequestConfig, ServerRequestConfigBuilder, ServerState,
};

/// Versioned ABI used exclusively by generated code.
#[doc(hidden)]
pub mod __macro {
    /// ABI version used by fusen-rs 0.9 generated code.
    pub mod v1 {
        pub use crate::client::ServiceClient;
        pub use crate::interface::spring_method;
        pub use crate::service::{
            IntoServerService, PreparedService, ServerInvocation, ServerService, method_not_found,
        };
        pub use crate::{
            ClientBuilder, ClientRuntime, Idempotency, Middleware, MiddlewareFuture, RpcError,
            RpcField, RpcFieldSource, RpcMessage, RpcRequest, RpcResponse, WireProtocol,
        };
        pub use fusen_contract::{MethodDescriptor, MethodId, ServiceDescriptor, ServiceSelector};
        pub use http;
    }
}

/// Registry SPI and all of its parameter and lifecycle types.
pub mod registry {
    pub use fusen_register::*;
}

/// Low-cardinality observability SPI and event types.
pub mod observability {
    pub use fusen_observability::*;
}
