use crate::protocol::fusen::request::{ArgumentValue, FusenRequest};
use fusen_contract::{MethodDescriptor, MethodId, ServiceDescriptor};
use http::{Extensions, HeaderMap};
use std::{collections::HashMap, time::Duration};
use tokio::time::Instant;

/// Request-scoped state passed through routing, middleware, and dispatch.
///
/// Bodies and transport details remain framework-owned. Middleware receives stable identity,
/// mutable headers and metadata, and typed extensions without relying on thread-local state.
pub struct RpcContext {
    request_id: String,
    service: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    deadline: Instant,
    metadata: HashMap<String, String>,
    extensions: Extensions,
    pub(crate) request: FusenRequest,
}

impl std::fmt::Debug for RpcContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RpcContext")
            .field("request_id", &self.request_id)
            .field("service", &self.service())
            .field("method", &self.method())
            .field("method_id", &self.method_id())
            .field("deadline", &self.deadline)
            .field("metadata", &self.metadata)
            .finish_non_exhaustive()
    }
}

impl RpcContext {
    pub(crate) fn new(
        request_id: String,
        service: &'static ServiceDescriptor,
        method: &'static MethodDescriptor,
        request: FusenRequest,
        deadline: Instant,
    ) -> Self {
        Self {
            request_id,
            service,
            method,
            deadline,
            metadata: HashMap::new(),
            extensions: Extensions::new(),
            request,
        }
    }

    /// Returns the invocation correlation identifier.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the static service identifier.
    pub fn service(&self) -> &str {
        self.service.selector().service_id()
    }

    /// Returns the static RPC method name.
    pub fn method(&self) -> &str {
        self.method.name()
    }

    /// Returns the declaration-order method identifier.
    pub const fn method_id(&self) -> MethodId {
        self.method.id()
    }

    /// Returns the absolute invocation deadline.
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Returns the time remaining before the invocation deadline.
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Returns request headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.request.headers
    }

    /// Returns mutable request headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.request.headers
    }

    /// Returns invocation metadata used by routers and load balancers.
    pub fn metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }

    /// Returns mutable invocation metadata used by routers and load balancers.
    pub fn metadata_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.metadata
    }

    /// Returns typed request-local extensions.
    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Returns mutable typed request-local extensions.
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    /// Framework entry used by generated service dispatch.
    #[doc(hidden)]
    pub fn __take_arguments(&mut self) -> Result<Vec<ArgumentValue>, crate::error::FusenError> {
        self.request.take_arguments(self.method)
    }

    /// Framework entry used by generated service dispatch.
    #[doc(hidden)]
    pub const fn __protocol(&self) -> fusen_contract::WireProtocol {
        self.request.protocol
    }
}
