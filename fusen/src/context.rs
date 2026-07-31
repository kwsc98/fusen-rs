use crate::runtime::{
    budget::{BudgetedWriteFailure, BudgetedWriter, ByteBudget, BytePermit},
    deadline::Deadline,
};
use bytes::Bytes;
use fusen_contract::{MethodDescriptor, ServiceDescriptor, ServiceInstance, WireProtocol};
use http::{Extensions, HeaderMap, StatusCode};
use serde_json::{Map, Value};
use std::{
    fmt,
    num::NonZeroU8,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
    time::Duration,
};

/// Named JSON values used by the versioned wire protocols.
#[derive(Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct RpcArguments(Map<String, Value>);

impl fmt::Debug for RpcArguments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RpcArguments")
            .field("field_count", &self.0.len())
            .finish()
    }
}

impl RpcArguments {
    /// Creates an empty argument object.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Deref for RpcArguments {
    type Target = Map<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RpcArguments {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Whether middleware is processing an outbound or inbound RPC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RpcSide {
    /// An outbound client invocation.
    Client,
    /// An inbound server invocation.
    Server,
}

/// The precise point at which middleware runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MiddlewareStage {
    /// Once around the complete logical client invocation.
    ClientCall,
    /// Once around each physical client transport attempt.
    ClientAttempt,
    /// After server admission and before the request body is polled.
    ServerHead,
    /// After server decoding and before the interface handler.
    ServerCall,
}

/// Framework metadata bound to an [`RpcCall`].
#[derive(Clone, Debug)]
pub struct CallInfo {
    request_id: String,
    protocol: WireProtocol,
    interface: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    deadline: Deadline,
}

impl CallInfo {
    /// Returns the validated correlation identifier.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the selected wire protocol.
    pub const fn protocol(&self) -> WireProtocol {
        self.protocol
    }

    /// Returns the immutable interface descriptor.
    pub const fn interface(&self) -> &'static ServiceDescriptor {
        self.interface
    }

    /// Returns the immutable method descriptor.
    pub const fn method(&self) -> &'static MethodDescriptor {
        self.method
    }

    /// Returns the remaining logical deadline.
    pub fn remaining(&self) -> Duration {
        self.deadline.remaining()
    }
}

/// Optional call metadata passed explicitly by generated clients and interface handlers.
#[derive(Clone, Debug, Default)]
pub struct RpcCall {
    headers: HeaderMap,
    extensions: Extensions,
    call_info: Option<CallInfo>,
}

impl RpcCall {
    /// Creates call metadata with empty headers and extensions.
    pub fn new() -> Self {
        Self {
            headers: HeaderMap::new(),
            extensions: Extensions::new(),
            call_info: None,
        }
    }

    /// Returns application request headers.
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns mutable application request headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Returns cloneable request extensions.
    pub const fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Returns mutable cloneable request extensions.
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    /// Returns framework call metadata when the request is bound to an invocation.
    pub const fn call_info(&self) -> Option<&CallInfo> {
        self.call_info.as_ref()
    }

    pub(crate) fn into_parts(self) -> (HeaderMap, Extensions) {
        (self.headers, self.extensions)
    }

    pub(crate) fn from_server(context: &RpcContext) -> Self {
        Self {
            headers: context.headers.clone(),
            extensions: context.extensions.clone(),
            call_info: Some(context.call_info()),
        }
    }
}

#[derive(Clone, Copy)]
enum ResponseSchemaOrigin {
    Unclassified,
    Declared {
        method: &'static MethodDescriptor,
        direction: crate::projection::ProjectionDirection,
    },
}

/// Budget-aware encoded JSON body carried through middleware and transport.
#[derive(Clone)]
pub struct RpcBody {
    bytes: Bytes,
    budget_permit: Option<Arc<BytePermit>>,
    schema_origin: ResponseSchemaOrigin,
}

impl fmt::Debug for RpcBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RpcBody")
            .field("length", &self.bytes.len())
            .finish()
    }
}

impl RpcBody {
    /// Returns the encoded JSON bytes.
    pub const fn as_bytes(&self) -> &Bytes {
        &self.bytes
    }

    /// Returns the encoded byte length.
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether the encoded body is empty.
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(crate) fn from_bytes(bytes: Bytes) -> Self {
        Self {
            bytes,
            budget_permit: None,
            schema_origin: ResponseSchemaOrigin::Unclassified,
        }
    }

    pub(crate) fn into_parts(self) -> (Bytes, Option<Arc<BytePermit>>) {
        (self.bytes, self.budget_permit)
    }

    pub(crate) fn hold_budget(&mut self, permit: BytePermit) {
        self.budget_permit = Some(Arc::new(permit));
    }
}

#[derive(Clone, Debug)]
struct ResponseRuntime {
    tracks_endpoint_breaker: bool,
    service_breaker_permit: Option<Arc<Mutex<Option<crate::resilience::breaker::BreakerPermit>>>>,
}

/// A typed successful RPC response.
#[derive(Clone)]
pub struct RpcResponse<T> {
    body: T,
    status: StatusCode,
    headers: HeaderMap,
    extensions: Extensions,
    attempts: u8,
    runtime: ResponseRuntime,
}

impl<T> fmt::Debug for RpcResponse<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RpcResponse")
            .field("body", &"<omitted>")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .field("extensions", &self.extensions)
            .field("attempts", &self.attempts)
            .field("runtime", &self.runtime)
            .finish()
    }
}

impl<T> RpcResponse<T> {
    /// Creates a `200 OK` response with empty headers and extensions.
    pub fn new(body: T) -> Self {
        Self {
            body,
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            extensions: Extensions::new(),
            attempts: 1,
            runtime: ResponseRuntime {
                tracks_endpoint_breaker: false,
                service_breaker_permit: None,
            },
        }
    }

    /// Returns the response body.
    pub const fn body(&self) -> &T {
        &self.body
    }

    /// Returns the mutable response body.
    pub fn body_mut(&mut self) -> &mut T {
        &mut self.body
    }

    /// Consumes the response and returns its body.
    pub fn into_body(self) -> T {
        self.body
    }

    /// Transforms the body while preserving response metadata.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> RpcResponse<U> {
        RpcResponse {
            body: map(self.body),
            status: self.status,
            headers: self.headers,
            extensions: self.extensions,
            attempts: self.attempts,
            runtime: self.runtime,
        }
    }

    /// Returns the successful HTTP status.
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Replaces the response status. Successful responses must remain 2xx.
    pub fn set_status(&mut self, status: StatusCode) -> Result<(), crate::RpcError> {
        if !status.is_success() {
            return Err(crate::RpcError::framework(
                crate::RpcCategory::InvalidArgument,
                "invalid_response_status",
                "RPC success response status must be 2xx",
            ));
        }
        self.status = status;
        Ok(())
    }

    /// Returns response headers.
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns mutable response headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Returns response extensions.
    pub const fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Returns mutable response extensions.
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    /// Returns the final number of physical attempts.
    pub const fn attempts(&self) -> u8 {
        self.attempts
    }

    pub(crate) fn into_parts(self) -> (T, StatusCode, HeaderMap, Extensions, u8) {
        (
            self.body,
            self.status,
            self.headers,
            self.extensions,
            self.attempts,
        )
    }

    pub(crate) fn set_attempts(&mut self, attempts: u8) {
        self.attempts = attempts.max(1);
    }
}

impl RpcResponse<RpcBody> {
    /// Projects the encoded result through declared response sensitivity metadata.
    ///
    /// Responses that did not originate from the generated response schema, methods without
    /// response metadata, invalid JSON, policy panics, and projection limit failures are omitted
    /// in full.
    pub fn sanitized_body(
        &self,
        method: &MethodDescriptor,
        sanitizer: &dyn crate::sensitive::Sanitizer,
    ) -> crate::sensitive::SanitizedValue {
        crate::projection::sanitize_response(
            method,
            self.body.as_bytes(),
            self.declared_schema_direction(method),
            sanitizer,
        )
    }

    pub(crate) fn success_with_budget<T: serde::Serialize>(
        value: T,
        limit: usize,
        wire_overhead: usize,
        budget: &Arc<ByteBudget>,
    ) -> Result<Self, crate::RpcError> {
        let mut writer = BudgetedWriter::new(limit, budget, wire_overhead)
            .map_err(|_| response_budget_exhausted())?;
        serde_json::to_writer(&mut writer, &value).map_err(|error| match writer.failure() {
            Some(BudgetedWriteFailure::LimitExceeded) => response_too_large(),
            Some(BudgetedWriteFailure::BudgetExhausted) => response_budget_exhausted(),
            None => crate::RpcError::internal("failed to serialize RPC response", error),
        })?;
        let (bytes, permit) = writer.into_parts();
        Ok(Self::new(RpcBody {
            bytes,
            budget_permit: Some(permit),
            schema_origin: ResponseSchemaOrigin::Unclassified,
        }))
    }

    pub(crate) fn from_json_bytes(bytes: Bytes) -> Self {
        Self::new(RpcBody::from_bytes(bytes))
    }

    pub(crate) fn result_bytes(&self) -> &Bytes {
        self.body.as_bytes()
    }

    pub(crate) fn into_wire_parts(self) -> (StatusCode, HeaderMap, Bytes, Option<Arc<BytePermit>>) {
        let (bytes, permit) = self.body.into_parts();
        (self.status, self.headers, bytes, permit)
    }

    pub(crate) fn hold_budget(&mut self, permit: BytePermit) {
        self.body.hold_budget(permit);
    }

    pub(crate) fn mark_declared_serialize_schema_origin(
        &mut self,
        method: &'static MethodDescriptor,
    ) {
        self.mark_declared_schema_origin(method, crate::projection::ProjectionDirection::Serialize);
    }

    pub(crate) fn mark_declared_deserialize_schema_origin(
        &mut self,
        method: &'static MethodDescriptor,
    ) {
        self.mark_declared_schema_origin(
            method,
            crate::projection::ProjectionDirection::Deserialize,
        );
    }

    fn mark_declared_schema_origin(
        &mut self,
        method: &'static MethodDescriptor,
        direction: crate::projection::ProjectionDirection,
    ) {
        self.body.schema_origin = ResponseSchemaOrigin::Declared { method, direction };
    }

    fn declared_schema_direction(
        &self,
        method: &MethodDescriptor,
    ) -> Option<crate::projection::ProjectionDirection> {
        match self.body.schema_origin {
            ResponseSchemaOrigin::Declared {
                method: origin,
                direction,
            } if std::ptr::eq(origin, method) => Some(direction),
            ResponseSchemaOrigin::Unclassified | ResponseSchemaOrigin::Declared { .. } => None,
        }
    }

    pub(crate) fn track_endpoint_breaker(&mut self) {
        self.runtime.tracks_endpoint_breaker = true;
    }

    pub(crate) const fn tracks_endpoint_breaker(&self) -> bool {
        self.runtime.tracks_endpoint_breaker
    }

    pub(crate) fn hold_service_breaker(
        &mut self,
        permit: crate::resilience::breaker::BreakerPermit,
    ) {
        self.runtime.service_breaker_permit = Some(Arc::new(Mutex::new(Some(permit))));
    }

    pub(crate) fn take_service_breaker(
        &mut self,
    ) -> Option<crate::resilience::breaker::BreakerPermit> {
        self.runtime
            .service_breaker_permit
            .as_ref()
            .and_then(|permit| {
                permit
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take()
            })
    }
}

/// Metadata and mutable state for one middleware position.
#[derive(Clone)]
pub struct RpcContext {
    side: RpcSide,
    stage: MiddlewareStage,
    request_id: String,
    protocol: WireProtocol,
    interface: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    deadline: Deadline,
    attempt: Option<NonZeroU8>,
    endpoint: Option<ServiceInstance>,
    headers: HeaderMap,
    extensions: Extensions,
    arguments: Option<RpcArguments>,
    response_limit: usize,
    response_wire_overhead: usize,
    response_budget: Arc<ByteBudget>,
}

impl fmt::Debug for RpcContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RpcContext")
            .field("side", &self.side)
            .field("stage", &self.stage)
            .field("request_id", &self.request_id)
            .field("protocol", &self.protocol)
            .field("interface", &self.interface)
            .field("method", &self.method)
            .field("deadline", &self.deadline)
            .field("attempt", &self.attempt)
            .field("endpoint", &self.endpoint)
            .field("headers", &self.headers)
            .field("extensions", &self.extensions)
            .field(
                "argument_count",
                &self.arguments.as_ref().map(|arguments| arguments.len()),
            )
            .field("response_limit", &self.response_limit)
            .field("response_wire_overhead", &self.response_wire_overhead)
            .field("response_budget", &self.response_budget)
            .finish()
    }
}

pub(crate) struct RpcContextParts {
    pub side: RpcSide,
    pub stage: MiddlewareStage,
    pub request_id: String,
    pub protocol: WireProtocol,
    pub interface: &'static ServiceDescriptor,
    pub method: &'static MethodDescriptor,
    pub deadline: Deadline,
    pub attempt: Option<NonZeroU8>,
    pub endpoint: Option<ServiceInstance>,
    pub headers: HeaderMap,
    pub extensions: Extensions,
    pub arguments: Option<RpcArguments>,
    pub response_limit: usize,
    pub response_wire_overhead: usize,
    pub response_budget: Arc<ByteBudget>,
}

impl RpcContext {
    pub(crate) fn new(parts: RpcContextParts) -> Self {
        Self {
            side: parts.side,
            stage: parts.stage,
            request_id: parts.request_id,
            protocol: parts.protocol,
            interface: parts.interface,
            method: parts.method,
            deadline: parts.deadline,
            attempt: parts.attempt,
            endpoint: parts.endpoint,
            headers: parts.headers,
            extensions: parts.extensions,
            arguments: parts.arguments,
            response_limit: parts.response_limit,
            response_wire_overhead: parts.response_wire_overhead,
            response_budget: parts.response_budget,
        }
    }

    /// Returns the client/server side of this middleware position.
    pub const fn side(&self) -> RpcSide {
        self.side
    }

    /// Returns the middleware execution stage.
    pub const fn stage(&self) -> MiddlewareStage {
        self.stage
    }

    /// Returns the validated correlation identifier.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the selected wire protocol.
    pub const fn protocol(&self) -> WireProtocol {
        self.protocol
    }

    /// Returns the static interface contract.
    pub const fn interface(&self) -> &'static ServiceDescriptor {
        self.interface
    }

    /// Returns the static method contract.
    pub const fn method(&self) -> &'static MethodDescriptor {
        self.method
    }

    /// Returns the remaining logical invocation budget.
    pub fn remaining(&self) -> Duration {
        self.deadline.remaining()
    }

    /// Returns application headers.
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns mutable application headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Returns cloneable invocation extensions.
    pub const fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    /// Returns mutable cloneable invocation extensions.
    pub fn extensions_mut(&mut self) -> &mut Extensions {
        &mut self.extensions
    }

    /// Returns decoded or encoded arguments when available at this stage.
    pub const fn arguments(&self) -> Option<&RpcArguments> {
        self.arguments.as_ref()
    }

    /// Projects available arguments through the method's declared sensitivity metadata.
    ///
    /// `ServerHead` contexts and other positions without arguments return an omitted value because
    /// the request body has not been consumed. Metadata gaps, policy panics, and projection limit
    /// failures also fail closed to the same representation.
    pub fn sanitized_arguments(
        &self,
        sanitizer: &dyn crate::sensitive::Sanitizer,
    ) -> crate::sensitive::SanitizedValue {
        let direction = match self.side {
            RpcSide::Client => crate::projection::ProjectionDirection::Serialize,
            RpcSide::Server => crate::projection::ProjectionDirection::Deserialize,
        };
        self.arguments.as_ref().map_or_else(
            crate::sensitive::SanitizedValue::omitted,
            |arguments| {
                crate::projection::sanitize_arguments(self.method, arguments, direction, sanitizer)
            },
        )
    }

    /// Returns the physical attempt number at attempt-scoped stages.
    pub const fn attempt(&self) -> Option<NonZeroU8> {
        self.attempt
    }

    /// Returns the selected endpoint at a client attempt stage.
    pub const fn endpoint(&self) -> Option<&ServiceInstance> {
        self.endpoint.as_ref()
    }

    /// Creates a successful, budget-controlled short-circuit response.
    pub fn respond<T: serde::Serialize>(
        &self,
        value: T,
    ) -> Result<RpcResponse<RpcBody>, crate::RpcError> {
        RpcResponse::success_with_budget(
            value,
            self.response_limit,
            self.response_wire_overhead,
            &self.response_budget,
        )
    }

    pub(crate) const fn deadline(&self) -> Deadline {
        self.deadline
    }

    pub(crate) fn set_stage(&mut self, stage: MiddlewareStage) {
        self.stage = stage;
    }

    pub(crate) fn set_attempt(&mut self, attempt: u8) {
        self.attempt = NonZeroU8::new(attempt.max(1));
    }

    pub(crate) fn set_endpoint(&mut self, endpoint: ServiceInstance) {
        self.endpoint = Some(endpoint);
    }

    pub(crate) fn take_arguments(&mut self) -> Option<RpcArguments> {
        self.arguments.take()
    }

    pub(crate) fn set_arguments(&mut self, arguments: RpcArguments) {
        self.arguments = Some(arguments);
    }

    pub(crate) fn call_info(&self) -> CallInfo {
        CallInfo {
            request_id: self.request_id.clone(),
            protocol: self.protocol,
            interface: self.interface,
            method: self.method,
            deadline: self.deadline,
        }
    }
}

fn response_too_large() -> crate::RpcError {
    crate::RpcError::framework(
        crate::RpcCategory::Internal,
        "response_too_large",
        "encoded RPC response exceeds the configured limit",
    )
}

fn response_budget_exhausted() -> crate::RpcError {
    crate::RpcError::framework(
        crate::RpcCategory::ResourceExhausted,
        "response_byte_budget_exhausted",
        "response byte budget is exhausted",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PolicySanitizer, runtime::budget::ByteBudget};
    use fusen_contract::{
        MethodId, MethodSensitivity, SensitiveArgument, SensitiveField, SensitiveShape,
        SensitivityKind, ServiceSelector,
    };
    use serde_json::json;
    use std::{sync::OnceLock, time::Duration};

    fn public_shape() -> SensitiveShape {
        SensitiveShape::Kind(SensitivityKind::PUBLIC)
    }

    fn secret_shape() -> SensitiveShape {
        SensitiveShape::Kind(SensitivityKind::SECRET)
    }

    fn payload_shape() -> SensitiveShape {
        SensitiveShape::Fields {
            serialize: &[
                const { SensitiveField::new("visible_out", public_shape) },
                const { SensitiveField::new("secret_out", secret_shape) },
            ],
            deserialize: &[
                const { SensitiveField::new("visible_in", public_shape) },
                const { SensitiveField::new("visible_out", secret_shape) },
            ],
        }
    }

    fn service() -> &'static ServiceDescriptor {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        SERVICE.get_or_init(|| {
            let method = MethodDescriptor::new(MethodId::new(0), "call", None)
                .unwrap()
                .with_sensitivity(MethodSensitivity::new(
                    vec![SensitiveArgument::new("request", payload_shape)],
                    Some(payload_shape),
                ));
            ServiceDescriptor::new(
                ServiceSelector::new("sensitive-context-test", None, None).unwrap(),
                vec![method],
            )
            .unwrap()
        })
    }

    fn context() -> RpcContext {
        let service = service();
        let mut arguments = RpcArguments::new();
        arguments.insert(
            "request".to_owned(),
            json!({"visible_in": "safe", "visible_out": "private-argument"}),
        );
        RpcContext::new(RpcContextParts {
            side: RpcSide::Server,
            stage: MiddlewareStage::ServerCall,
            request_id: "request-1".to_owned(),
            protocol: WireProtocol::FusenV1,
            interface: service,
            method: service.method(MethodId::new(0)).unwrap(),
            deadline: Deadline::after(Duration::from_secs(1)),
            attempt: None,
            endpoint: None,
            headers: HeaderMap::new(),
            extensions: Extensions::new(),
            arguments: Some(arguments),
            response_limit: 4096,
            response_wire_overhead: 0,
            response_budget: ByteBudget::new(16 * 1024),
        })
    }

    #[test]
    fn payload_carrier_debug_never_expands_values() {
        struct PrivateDebug;

        impl fmt::Debug for PrivateDebug {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("private-typed-body")
            }
        }

        let context = context();
        let arguments_debug = format!("{:?}", context.arguments().unwrap());
        let context_debug = format!("{context:?}");
        let body_debug = format!(
            "{:?}",
            RpcBody::from_bytes(Bytes::from_static(b"private-encoded-body"))
        );
        let response_debug = format!("{:?}", RpcResponse::new(PrivateDebug));

        for debug in [arguments_debug, context_debug, body_debug, response_debug] {
            assert!(!debug.contains("private-"));
        }
    }

    #[test]
    fn declared_responses_project_but_context_short_circuits_do_not() {
        let context = context();
        let method = context.method();
        let policy = PolicySanitizer::default();

        assert_eq!(
            serde_json::to_value(context.sanitized_arguments(&policy)).unwrap(),
            json!({"request": {"visible_in": "safe", "visible_out": "<redacted>"}})
        );

        let short_circuit = context
            .respond(json!({"visible_out": "safe", "secret_out": "private-response"}))
            .unwrap();
        assert!(short_circuit.sanitized_body(method, &policy).is_omitted());

        let mut declared = RpcResponse::from_json_bytes(Bytes::from_static(
            br#"{"visible_out":"safe","secret_out":"private-response"}"#,
        ));
        declared.mark_declared_serialize_schema_origin(method);
        assert_eq!(
            serde_json::to_value(declared.sanitized_body(method, &policy)).unwrap(),
            json!({"visible_out": "safe", "secret_out": "<redacted>"})
        );

        let mut received = RpcResponse::from_json_bytes(Bytes::from_static(
            br#"{"visible_in":"safe","visible_out":"private-response"}"#,
        ));
        received.mark_declared_deserialize_schema_origin(method);
        assert_eq!(
            serde_json::to_value(received.sanitized_body(method, &policy)).unwrap(),
            json!({"visible_in": "safe", "visible_out": "<redacted>"})
        );

        let wrong_method = MethodDescriptor::new(MethodId::new(1), "other", None)
            .unwrap()
            .with_sensitivity(MethodSensitivity::new(vec![], Some(payload_shape)));
        assert!(declared.sanitized_body(&wrong_method, &policy).is_omitted());

        let replacement = context
            .respond(json!({"visible_out": "private-replacement"}))
            .unwrap()
            .into_body();
        *declared.body_mut() = replacement;
        assert!(declared.sanitized_body(method, &policy).is_omitted());

        let mut declared =
            RpcResponse::from_json_bytes(Bytes::from_static(br#"{"visible_out":"safe"}"#));
        declared.mark_declared_serialize_schema_origin(method);
        let replacement = context
            .respond(json!({"visible_out": "private-map-replacement"}))
            .unwrap()
            .into_body();
        let mapped = declared.map(|_| replacement);
        assert!(mapped.sanitized_body(method, &policy).is_omitted());
    }
}
