use crate::runtime::{
    budget::{BudgetedWriteFailure, BudgetedWriter, ByteBudget, BytePermit},
    deadline::Deadline,
};
use bytes::Bytes;
use fusen_contract::{HttpBindingId, MethodDescriptor, ServiceDescriptor, ServiceInstance};
use http::{Extensions, HeaderMap, StatusCode};
use serde_json::{Map, Value};
use std::{
    fmt,
    num::NonZeroU8,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
    time::Duration,
};

/// Named JSON values used by the HTTP binding.
#[derive(Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Arguments(Map<String, Value>);

impl fmt::Debug for Arguments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Arguments")
            .field("field_count", &self.0.len())
            .finish()
    }
}

impl Arguments {
    /// Creates an empty argument object.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Deref for Arguments {
    type Target = Map<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Arguments {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Whether an interceptor is processing an outbound or inbound service invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Side {
    /// An outbound client invocation.
    Client,
    /// An inbound server invocation.
    Server,
}

/// The precise point at which interceptor runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InterceptionStage {
    /// Once around the complete logical client invocation.
    ClientCall,
    /// Once around each physical client transport attempt.
    ClientAttempt,
    /// After server admission and before the request body is polled.
    ServerHead,
    /// After server decoding and before the interface handler.
    ServerCall,
}

/// Framework metadata bound to a [`Call`].
#[derive(Clone)]
pub struct CallInfo {
    request_id: String,
    binding_id: HttpBindingId,
    http_version: Option<http::Version>,
    interface: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    deadline: Deadline,
}

impl fmt::Debug for CallInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CallInfo")
            .field("request_id", &self.request_id)
            .field("binding_id", &self.binding_id)
            .field("http_version", &self.http_version)
            .field("interface", &self.interface.identity())
            .field("method", &self.method.invocation_name())
            .finish()
    }
}

impl CallInfo {
    /// Returns the validated correlation identifier.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the selected HTTP binding.
    pub const fn binding_id(&self) -> &HttpBindingId {
        &self.binding_id
    }

    /// Returns the selected or negotiated HTTP version when transport has been bound.
    pub const fn http_version(&self) -> Option<http::Version> {
        self.http_version
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
#[derive(Clone, Default)]
pub struct Call {
    headers: HeaderMap,
    extensions: Extensions,
    call_info: Option<CallInfo>,
}

impl fmt::Debug for Call {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Call")
            .field("header_count", &self.headers.len())
            .field("extension_count", &self.extensions.len())
            .field("call_info", &self.call_info)
            .finish()
    }
}

impl Call {
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

    pub(crate) fn from_server(context: &Context) -> Self {
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

/// Budget-aware encoded JSON body carried through interceptor and transport.
#[derive(Clone)]
pub struct Body {
    bytes: Bytes,
    budget_permit: Option<Arc<BytePermit>>,
    schema_origin: ResponseSchemaOrigin,
}

impl fmt::Debug for Body {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Body")
            .field("length", &self.bytes.len())
            .finish()
    }
}

impl Body {
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

    /// Creates an encoded body from already bounded bytes.
    pub fn from_bytes(bytes: Bytes) -> Self {
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

pub(crate) trait ResponseAttemptCompletion: Send + Sync {
    fn seal_duration(&self, duration: Duration);

    fn finish(&self, failure: Option<crate::resilience::FailureClass>);
}

#[derive(Clone)]
struct ResponseRuntime {
    wire_origin: bool,
    tracks_endpoint_breaker: bool,
    service_breaker_permit: Option<Arc<Mutex<Option<crate::resilience::breaker::BreakerPermit>>>>,
    attempt_completion: Option<Arc<dyn ResponseAttemptCompletion>>,
}

impl fmt::Debug for ResponseRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResponseRuntime")
            .field("wire_origin", &self.wire_origin)
            .field("tracks_endpoint_breaker", &self.tracks_endpoint_breaker)
            .field(
                "has_service_breaker_permit",
                &self.service_breaker_permit.is_some(),
            )
            .field("has_attempt_completion", &self.attempt_completion.is_some())
            .finish()
    }
}

/// A typed successful service invocation response.
#[derive(Clone)]
pub struct Response<T> {
    body: T,
    status: StatusCode,
    headers: HeaderMap,
    extensions: Extensions,
    attempts: u8,
    runtime: ResponseRuntime,
}

impl<T> fmt::Debug for Response<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Response")
            .field("status", &self.status)
            .field("header_count", &self.headers.len())
            .field("extension_count", &self.extensions.len())
            .field("attempts", &self.attempts)
            .finish()
    }
}

impl<T> Response<T> {
    /// Creates a `200 OK` response with empty headers and extensions.
    pub fn new(body: T) -> Self {
        Self {
            body,
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            extensions: Extensions::new(),
            attempts: 0,
            runtime: ResponseRuntime {
                wire_origin: false,
                tracks_endpoint_breaker: false,
                service_breaker_permit: None,
                attempt_completion: None,
            },
        }
    }

    /// Returns the response body.
    pub const fn body(&self) -> &T {
        &self.body
    }

    /// Returns the mutable response body and marks it as locally transformed.
    ///
    /// A typed decode failure after mutable body access is attributed to the interceptor rather
    /// than to the remote service response.
    pub fn body_mut(&mut self) -> &mut T {
        self.runtime.wire_origin = false;
        &mut self.body
    }

    /// Consumes the response and returns its body.
    pub fn into_body(self) -> T {
        self.body
    }

    /// Transforms the body while preserving response metadata.
    ///
    /// The transformed body is local even when the original body came from the wire. Status,
    /// headers, extensions, physical-attempt accounting, and breaker completion state are kept.
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Response<U> {
        let mut runtime = self.runtime;
        runtime.wire_origin = false;
        Response {
            body: map(self.body),
            status: self.status,
            headers: self.headers,
            extensions: self.extensions,
            attempts: self.attempts,
            runtime,
        }
    }

    /// Returns the successful HTTP status.
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Replaces the response status. Successful responses must remain 2xx.
    pub fn set_status(&mut self, status: StatusCode) -> Result<(), crate::Error> {
        if !status.is_success() {
            return Err(crate::Error::framework(
                crate::ErrorCategory::InvalidArgument,
                "invalid_response_status",
                "service invocation success response status must be 2xx",
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
        self.attempts = attempts;
    }

    pub(crate) fn hold_attempt_completion(
        &mut self,
        completion: Arc<dyn ResponseAttemptCompletion>,
    ) {
        self.runtime.attempt_completion = Some(completion);
    }

    pub(crate) fn mark_wire_origin(&mut self) {
        self.runtime.wire_origin = true;
    }

    pub(crate) const fn is_wire_origin(&self) -> bool {
        self.runtime.wire_origin
    }

    pub(crate) fn seal_attempt_duration(&self, duration: Duration) {
        if let Some(completion) = &self.runtime.attempt_completion {
            completion.seal_duration(duration);
        }
    }

    pub(crate) fn finish_attempt(&mut self, failure: Option<crate::resilience::FailureClass>) {
        if let Some(completion) = self.runtime.attempt_completion.take() {
            completion.finish(failure);
        }
    }
}

impl Response<Body> {
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
    ) -> Result<Self, crate::Error> {
        let mut writer = BudgetedWriter::new(limit, budget, wire_overhead)
            .map_err(|_| response_budget_exhausted())?;
        serde_json::to_writer(&mut writer, &value).map_err(|error| match writer.failure() {
            Some(BudgetedWriteFailure::LimitExceeded) => response_too_large(),
            Some(BudgetedWriteFailure::BudgetExhausted) => response_budget_exhausted(),
            None => crate::Error::internal("failed to serialize invocation response", error),
        })?;
        let (bytes, permit) = writer.into_parts();
        Ok(Self::new(Body {
            bytes,
            budget_permit: Some(permit),
            schema_origin: ResponseSchemaOrigin::Unclassified,
        }))
    }

    pub(crate) fn from_json_bytes(bytes: Bytes) -> Self {
        Self::new(Body::from_bytes(bytes))
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

/// Metadata and mutable state for one interceptor position.
#[derive(Clone)]
pub struct Context {
    side: Side,
    stage: InterceptionStage,
    request_id: String,
    binding_id: HttpBindingId,
    http_version: Option<http::Version>,
    interface: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    deadline: Deadline,
    attempt: Option<NonZeroU8>,
    endpoint: Option<ServiceInstance>,
    headers: HeaderMap,
    extensions: Extensions,
    arguments: Option<Arguments>,
    response_limit: usize,
    response_wire_overhead: usize,
    response_budget: Arc<ByteBudget>,
}

impl fmt::Debug for Context {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Context")
            .field("side", &self.side)
            .field("stage", &self.stage)
            .field("request_id", &self.request_id)
            .field("binding_id", &self.binding_id)
            .field("http_version", &self.http_version)
            .field("interface", &self.interface.identity())
            .field("method", &self.method.invocation_name())
            .field("attempt", &self.attempt)
            .field(
                "endpoint_instance_id",
                &self
                    .endpoint
                    .as_ref()
                    .map(fusen_contract::ServiceInstance::instance_id),
            )
            .field("header_count", &self.headers.len())
            .field("extension_count", &self.extensions.len())
            .field(
                "argument_count",
                &self.arguments.as_ref().map(|arguments| arguments.len()),
            )
            .finish()
    }
}

pub(crate) struct ContextParts {
    pub side: Side,
    pub stage: InterceptionStage,
    pub request_id: String,
    pub binding_id: HttpBindingId,
    pub http_version: Option<http::Version>,
    pub interface: &'static ServiceDescriptor,
    pub method: &'static MethodDescriptor,
    pub deadline: Deadline,
    pub attempt: Option<NonZeroU8>,
    pub endpoint: Option<ServiceInstance>,
    pub headers: HeaderMap,
    pub extensions: Extensions,
    pub arguments: Option<Arguments>,
    pub response_limit: usize,
    pub response_wire_overhead: usize,
    pub response_budget: Arc<ByteBudget>,
}

impl Context {
    pub(crate) fn new(parts: ContextParts) -> Self {
        Self {
            side: parts.side,
            stage: parts.stage,
            request_id: parts.request_id,
            binding_id: parts.binding_id,
            http_version: parts.http_version,
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

    /// Returns the client/server side of this interceptor position.
    pub const fn side(&self) -> Side {
        self.side
    }

    /// Returns the interceptor execution stage.
    pub const fn stage(&self) -> InterceptionStage {
        self.stage
    }

    /// Returns the validated correlation identifier.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the selected HTTP binding.
    pub const fn binding_id(&self) -> &HttpBindingId {
        &self.binding_id
    }

    /// Returns the selected or negotiated HTTP version at transport-bound stages.
    pub const fn http_version(&self) -> Option<http::Version> {
        self.http_version
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
    pub const fn arguments(&self) -> Option<&Arguments> {
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
            Side::Client => crate::projection::ProjectionDirection::Serialize,
            Side::Server => crate::projection::ProjectionDirection::Deserialize,
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
    pub fn respond<T: serde::Serialize>(&self, value: T) -> Result<Response<Body>, crate::Error> {
        Response::success_with_budget(
            value,
            self.response_limit,
            self.response_wire_overhead,
            &self.response_budget,
        )
    }

    pub(crate) const fn deadline(&self) -> Deadline {
        self.deadline
    }

    pub(crate) fn set_stage(&mut self, stage: InterceptionStage) {
        self.stage = stage;
    }

    pub(crate) fn set_attempt(&mut self, attempt: u8) {
        self.attempt = NonZeroU8::new(attempt.max(1));
    }

    pub(crate) fn set_endpoint(&mut self, endpoint: ServiceInstance) {
        self.endpoint = Some(endpoint);
    }

    pub(crate) fn set_http_version(&mut self, version: http::Version) {
        self.http_version = Some(version);
    }

    pub(crate) fn take_arguments(&mut self) -> Option<Arguments> {
        self.arguments.take()
    }

    pub(crate) fn set_arguments(&mut self, arguments: Arguments) {
        self.arguments = Some(arguments);
    }

    pub(crate) fn call_info(&self) -> CallInfo {
        CallInfo {
            request_id: self.request_id.clone(),
            binding_id: self.binding_id.clone(),
            http_version: self.http_version,
            interface: self.interface,
            method: self.method,
            deadline: self.deadline,
        }
    }
}

fn response_too_large() -> crate::Error {
    crate::Error::framework(
        crate::ErrorCategory::Internal,
        "response_too_large",
        "encoded invocation response exceeds the configured limit",
    )
}

fn response_budget_exhausted() -> crate::Error {
    crate::Error::framework(
        crate::ErrorCategory::ResourceExhausted,
        "response_byte_budget_exhausted",
        "response byte budget is exhausted",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PolicySanitizer, runtime::budget::ByteBudget};
    use fusen_contract::{
        EndpointCapabilities, HttpBindingId, HttpOperation, InstanceId, Metadata, MethodId,
        MethodSensitivity, SensitiveArgument, SensitiveField, SensitiveShape, SensitivityKind,
        ServiceInstance, ServiceSelector, ServiceWeight,
    };
    use http::Method;
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
            let operation = HttpOperation::new(
                Method::POST,
                "/call",
                Vec::new(),
                "application/json",
                "application/json",
            )
            .unwrap();
            let method = MethodDescriptor::new(MethodId::new(0), "call", operation)
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

    fn context() -> Context {
        let service = service();
        let mut arguments = Arguments::new();
        arguments.insert(
            "request".to_owned(),
            json!({"visible_in": "safe", "visible_out": "private-argument"}),
        );
        Context::new(ContextParts {
            side: Side::Server,
            stage: InterceptionStage::ServerCall,
            request_id: "request-1".to_owned(),
            binding_id: HttpBindingId::default(),
            http_version: Some(http::Version::HTTP_11),
            interface: service,
            method: service.method(MethodId::new(0)).unwrap(),
            deadline: Deadline::after(Duration::from_secs(1)),
            attempt: None,
            endpoint: Some(
                ServiceInstance::new(
                    InstanceId::new("sensitive-context-instance").unwrap(),
                    "http://127.0.0.1:8080".parse().unwrap(),
                    EndpointCapabilities::default(),
                    ServiceWeight::default(),
                )
                .with_metadata(Metadata::from([(
                    "credential".into(),
                    "private-context-metadata-token".into(),
                )]))
                .unwrap(),
            ),
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
        #[derive(Clone)]
        struct PrivateDebug;

        impl fmt::Debug for PrivateDebug {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("private-typed-body")
            }
        }

        let mut context = context();
        context.headers_mut().insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_static("private-context-header"),
        );
        context.headers_mut().insert(
            http::header::COOKIE,
            http::HeaderValue::from_static("session=private-cookie-header"),
        );
        context.extensions_mut().insert(PrivateDebug);

        let call = Call::from_server(&context);
        let mut response = Response::new(PrivateDebug);
        response.headers_mut().insert(
            http::header::SET_COOKIE,
            http::HeaderValue::from_static("session=private-response-header"),
        );
        response.extensions_mut().insert(PrivateDebug);

        let arguments_debug = format!("{:?}", context.arguments().unwrap());
        let context_debug = format!("{context:?}");
        let call_debug = format!("{call:?}");
        let body_debug = format!(
            "{:?}",
            Body::from_bytes(Bytes::from_static(b"private-encoded-body"))
        );
        let response_debug = format!("{response:?}");

        assert!(!context_debug.contains("deadline"));
        assert!(!context_debug.contains("response_budget"));
        assert!(!response_debug.contains("runtime"));

        for debug in [
            arguments_debug,
            context_debug,
            call_debug,
            body_debug,
            response_debug,
        ] {
            assert!(!debug.contains("private-"));
        }
    }

    #[test]
    fn response_attempts_are_exact_and_preserved_by_map() {
        let mut response = Response::new("body");
        assert_eq!(response.attempts(), 0);

        response.set_attempts(2);
        assert_eq!(response.attempts(), 2);

        response.set_attempts(0);
        assert_eq!(response.attempts(), 0);
        assert_eq!(response.map(str::len).attempts(), 0);
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

        let mut declared = Response::from_json_bytes(Bytes::from_static(
            br#"{"visible_out":"safe","secret_out":"private-response"}"#,
        ));
        declared.mark_declared_serialize_schema_origin(method);
        assert_eq!(
            serde_json::to_value(declared.sanitized_body(method, &policy)).unwrap(),
            json!({"visible_out": "safe", "secret_out": "<redacted>"})
        );

        let mut received = Response::from_json_bytes(Bytes::from_static(
            br#"{"visible_in":"safe","visible_out":"private-response"}"#,
        ));
        received.mark_declared_deserialize_schema_origin(method);
        assert_eq!(
            serde_json::to_value(received.sanitized_body(method, &policy)).unwrap(),
            json!({"visible_in": "safe", "visible_out": "<redacted>"})
        );

        let wrong_method = MethodDescriptor::new(
            MethodId::new(1),
            "other",
            HttpOperation::new(
                Method::POST,
                "/other",
                Vec::new(),
                "application/json",
                "application/json",
            )
            .unwrap(),
        )
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
            Response::from_json_bytes(Bytes::from_static(br#"{"visible_out":"safe"}"#));
        declared.mark_declared_serialize_schema_origin(method);
        let replacement = context
            .respond(json!({"visible_out": "private-map-replacement"}))
            .unwrap()
            .into_body();
        let mapped = declared.map(|_| replacement);
        assert!(mapped.sanitized_body(method, &policy).is_omitted());
    }
}
