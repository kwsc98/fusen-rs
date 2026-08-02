#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Production-oriented Rust microservice and service-invocation runtime.

extern crate self as fusen_rs;

mod client;
/// Client-side HTTP request and response codec extension APIs.
pub mod codec;
mod context;
mod error;
pub(crate) use error::RemoteErrorParts;
/// Shared client/server interceptor API.
pub mod interceptor;
/// Interface parameter schema and argument encoding contracts.
pub mod interface;
/// Client routing, load balancing, and retry policy APIs.
pub mod policy;
mod projection;
mod resilience;
mod runtime;
/// Policy-driven safe projections for interceptor and application diagnostics.
pub mod sensitive;
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
pub use codec::{
    BufferedResponse, EncodedRequest, ErrorDecoder, RequestEncoder, RequestEncoding,
    ResponseDecoder,
};
pub use context::{Arguments, Body, Call, CallInfo, Context, InterceptionStage, Response, Side};
pub use error::{
    ClientError, ClientErrorKind, ConfigValidationError, ConfigValidationErrorKind, Error,
    ErrorCategory, ErrorCode, ErrorConstructionError, ErrorDetails, ErrorKind, ErrorOrigin,
    InvalidErrorCode, RetryHint, ServerError, ServerErrorKind,
};
pub use fusen_contract as contract;
pub use fusen_contract::{
    EndpointCapabilities, HTTP_JSON_V1, HttpBindingId, HttpOperation, HttpParameter,
    HttpParameterCardinality, HttpParameterSource, HttpVersionPolicy, HttpVersionSet,
    MethodSensitivity, SensitiveArgument, SensitiveField, SensitiveFields, SensitiveShape,
    SensitiveShapeResolver, SensitivityKind,
};
pub use fusen_observability::{MetricsRecorder, NoopMetricsRecorder};
pub use fusen_procedural_macro::{interface, method};
pub use fusen_register::{RegistrationHandle, Registry, SubscriptionHandle};
pub use interceptor::{Interceptor, InterceptorFuture, InterceptorResult, Next};
pub use policy::{InstanceRouter, InstanceSnapshot, LoadBalancer, RouteRequest, WeightedRandom};
pub use resilience::{FailureClass, RetryDecision, RetryDecisionContext, RetryPolicy};
pub use sensitive::{
    PolicySanitizer, ProjectionLimits, Sanitization, SanitizationContext, SanitizationTarget,
    SanitizedValue, Sanitizer,
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
        pub use crate::interface::{ArgumentField, ArgumentSource, encode_argument, http_method};
        pub use crate::service::{
            IntoServerService, PreparedService, ServerInvocation, ServerService, method_not_found,
        };
        pub use crate::{
            Arguments, Call, ClientBuilder, ClientRuntime, Error, Interceptor, InterceptorFuture,
            Response,
        };
        pub use fusen_contract::{
            MethodDescriptor, MethodId, MethodSensitivity, SensitiveArgument, SensitiveField,
            SensitiveFields, SensitiveShape, SensitiveShapeResolver, SensitivityKind,
            ServiceDescriptor, ServiceSelector,
        };
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
