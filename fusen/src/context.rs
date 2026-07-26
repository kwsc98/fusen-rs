use crate::runtime::{
    budget::{BudgetedWriteFailure, BudgetedWriter, ByteBudget, BytePermit},
    deadline::Deadline,
};
use bytes::Bytes;
use fusen_contract::{MethodDescriptor, ServiceDescriptor, WireProtocol};
use http::HeaderMap;
use serde_json::{Map, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

/// Named JSON arguments used by both generated protocol adapters.
#[doc(hidden)]
pub type Arguments = Map<String, Value>;

/// Metadata and mutable JSON arguments for one logical RPC invocation.
#[derive(Clone, Debug)]
pub struct RpcContext {
    request_id: String,
    protocol: WireProtocol,
    service: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    deadline: Deadline,
    attempt: u8,
    headers: HeaderMap,
    arguments: Arguments,
    response_limit: usize,
    response_wire_overhead: usize,
    response_budget: Arc<ByteBudget>,
}

pub(crate) struct RpcContextParts {
    pub request_id: String,
    pub protocol: WireProtocol,
    pub service: &'static ServiceDescriptor,
    pub method: &'static MethodDescriptor,
    pub deadline: Deadline,
    pub attempt: u8,
    pub headers: HeaderMap,
    pub arguments: Arguments,
    pub response_limit: usize,
    pub response_wire_overhead: usize,
    pub response_budget: Arc<ByteBudget>,
}

impl RpcContext {
    pub(crate) fn new(parts: RpcContextParts) -> Self {
        Self {
            request_id: parts.request_id,
            protocol: parts.protocol,
            service: parts.service,
            method: parts.method,
            deadline: parts.deadline,
            attempt: parts.attempt.max(1),
            headers: parts.headers,
            arguments: parts.arguments,
            response_limit: parts.response_limit,
            response_wire_overhead: parts.response_wire_overhead,
            response_budget: parts.response_budget,
        }
    }

    /// Returns the validated correlation identifier shared by all attempts.
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the selected wire protocol.
    pub const fn protocol(&self) -> WireProtocol {
        self.protocol
    }

    /// Returns the static service contract.
    pub const fn service(&self) -> &'static ServiceDescriptor {
        self.service
    }

    /// Returns the static method contract.
    pub const fn method(&self) -> &'static MethodDescriptor {
        self.method
    }

    /// Returns the current physical attempt number, starting at one.
    pub const fn attempt(&self) -> u8 {
        self.attempt
    }

    /// Returns the remaining logical invocation budget.
    pub fn remaining(&self) -> Duration {
        self.deadline.remaining()
    }

    /// Returns immutable application headers after framework control headers were validated.
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns mutable application headers.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// Returns the named JSON arguments.
    pub const fn arguments(&self) -> &Arguments {
        &self.arguments
    }

    /// Returns mutable named JSON arguments.
    pub fn arguments_mut(&mut self) -> &mut Arguments {
        &mut self.arguments
    }

    /// Creates a successful middleware response within this runtime's response limits.
    ///
    /// The byte budget is reserved incrementally before the JSON buffer grows and remains held
    /// until the response is sent, consumed by the generated client, or dropped.
    pub fn respond<T: serde::Serialize>(&self, value: T) -> Result<RpcResponse, crate::RpcError> {
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

    pub(crate) fn set_attempt(&mut self, attempt: u8) {
        self.attempt = attempt.max(1);
    }
}

/// Framework response passed through middleware before wire encoding.
#[derive(Clone, Debug)]
pub struct RpcResponse {
    status: http::StatusCode,
    headers: HeaderMap,
    result: Bytes,
    budget_permit: Option<Arc<BytePermit>>,
    tracks_endpoint_breaker: bool,
    service_breaker_permit: Option<Arc<Mutex<Option<crate::resilience::breaker::BreakerPermit>>>>,
    attempts: u8,
}

impl RpcResponse {
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
        let (result, permit) = writer.into_parts();
        Ok(Self {
            status: http::StatusCode::OK,
            headers: HeaderMap::new(),
            result,
            budget_permit: Some(permit),
            tracks_endpoint_breaker: false,
            service_breaker_permit: None,
            attempts: 1,
        })
    }

    pub(crate) fn from_json_bytes(result: Bytes) -> Self {
        Self {
            status: http::StatusCode::OK,
            headers: HeaderMap::new(),
            result,
            budget_permit: None,
            tracks_endpoint_breaker: false,
            service_breaker_permit: None,
            attempts: 1,
        }
    }

    /// Returns the response status.
    pub const fn status(&self) -> http::StatusCode {
        self.status
    }

    /// Replaces the response status. Success responses must remain in the 2xx class.
    pub fn set_status(&mut self, status: http::StatusCode) -> Result<(), crate::RpcError> {
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

    /// Deserializes the JSON result without consuming the response.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, crate::RpcError> {
        serde_json::from_slice(&self.result)
            .map_err(|error| crate::RpcError::internal("failed to decode RPC response", error))
    }

    pub(crate) const fn result_bytes(&self) -> &Bytes {
        &self.result
    }

    pub(crate) fn into_wire_parts(
        self,
    ) -> (http::StatusCode, HeaderMap, Bytes, Option<Arc<BytePermit>>) {
        (self.status, self.headers, self.result, self.budget_permit)
    }

    pub(crate) fn hold_budget(&mut self, permit: BytePermit) {
        self.budget_permit = Some(Arc::new(permit));
    }

    pub(crate) fn track_endpoint_breaker(&mut self) {
        self.tracks_endpoint_breaker = true;
    }

    pub(crate) const fn tracks_endpoint_breaker(&self) -> bool {
        self.tracks_endpoint_breaker
    }

    pub(crate) fn hold_service_breaker(
        &mut self,
        permit: crate::resilience::breaker::BreakerPermit,
    ) {
        self.service_breaker_permit = Some(Arc::new(Mutex::new(Some(permit))));
    }

    pub(crate) fn take_service_breaker(
        &mut self,
    ) -> Option<crate::resilience::breaker::BreakerPermit> {
        self.service_breaker_permit.as_ref().and_then(|permit| {
            permit
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
        })
    }

    pub(crate) fn set_attempts(&mut self, attempts: u8) {
        self.attempts = attempts.max(1);
    }

    pub(crate) const fn attempts(&self) -> u8 {
        self.attempts
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
    use fusen_contract::{
        Idempotency, MethodDescriptor, MethodId, ServiceDescriptor, ServiceSelector,
    };
    use std::{sync::OnceLock, time::Duration};

    fn descriptor() -> &'static ServiceDescriptor {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        SERVICE.get_or_init(|| {
            ServiceDescriptor::new(
                ServiceSelector::new("response-budget-test", None, None).unwrap(),
                vec![
                    MethodDescriptor::new(MethodId::new(0), "call", Idempotency::None, None)
                        .unwrap(),
                ],
            )
            .unwrap()
        })
    }

    fn context(budget: Arc<ByteBudget>) -> RpcContext {
        let service = descriptor();
        RpcContext::new(RpcContextParts {
            request_id: "budget-test".to_owned(),
            protocol: WireProtocol::FusenV1,
            service,
            method: service.method(MethodId::new(0)).unwrap(),
            deadline: Deadline::after(Duration::from_secs(1)),
            attempt: 1,
            headers: HeaderMap::new(),
            arguments: Arguments::new(),
            response_limit: 15,
            response_wire_overhead: 11,
            response_budget: budget,
        })
    }

    #[test]
    fn middleware_response_reserves_before_allocation_and_holds_until_drop() {
        let budget = ByteBudget::new(15);
        let response = context(budget.clone()).respond("ok").unwrap();
        assert_eq!(budget.used(), 15);
        drop(response);
        assert_eq!(budget.used(), 0);

        let exhausted = ByteBudget::new(14);
        let error = context(exhausted.clone()).respond("ok").unwrap_err();
        assert_eq!(error.code().as_str(), "response_byte_budget_exhausted");
        assert_eq!(exhausted.used(), 0);
    }
}
