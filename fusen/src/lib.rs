#![warn(missing_docs)]
#![doc = include_str!("../../README.md")]
//! Reliable JSON RPC runtime for HTTP/1.1 and HTTP/2 services.

extern crate self as fusen_rs;

#[path = "client/mod.rs"]
mod client_impl;
/// Advanced client-side cluster extensions.
pub mod client {
    /// Immutable instance routing and load-balancing contracts.
    pub mod cluster {
        pub use crate::client_impl::cluster::{InstanceSnapshot, LoadBalancer, Router};
    }
}
/// Stable framework errors and RFC 9457 response types.
pub mod error;
mod filter;
mod invocation;
mod protocol;
mod server;

pub use client_impl::{
    ClientConfig, ClientRuntime, ClientRuntimeBuilder, Http1PoolConfig, Http2PoolConfig,
};
pub use error::{FusenError, ProblemDetails};
pub use filter::{Middleware, Next, RpcResult};
/// Stable service registration and wire protocol contracts.
pub use fusen_contract as contract;
pub use invocation::{
    InvocationFinish, InvocationObserver, InvocationOutcome, InvocationPhase, InvocationSide,
    InvocationStart,
};
pub use protocol::fusen::{context::RpcContext, response::RpcResponse};
pub use server::{Server, ServerConfig};

mod request_id;
pub use fusen_procedural_macro::{asset, fusen_service, fusen_trait};

/// Framework internals referenced by generated code.
#[doc(hidden)]
pub mod __private {
    pub use crate::client_impl::{ServiceClient, ServiceClientBuilder};
    pub use crate::server::rpc::{RegisteredRpcService, RpcService, RpcServiceInfo};
    pub use crate::{
        filter::__benchmark_middleware,
        protocol::codec::{FusenHttpCodec, RequestCodec},
        protocol::fusen::request::{FusenRequest, Path},
        protocol::fusen::response::RpcResponse,
        server::{IntoServerService, PreparedService, ServerService},
    };
    pub use fusen_contract::{
        BoxFuture, MethodDescriptor, MethodId, ParameterDescriptor, ParameterSource,
        ServiceDescriptor,
    };
    pub use http;
    pub use serde_json;
}
