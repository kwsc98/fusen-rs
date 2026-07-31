#![allow(dead_code, missing_docs)]
#![forbid(unsafe_code)]

use bytes::Bytes;
use http::{HeaderMap, StatusCode};
use serde_json::{Map, Value};
use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, LazyLock},
};

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct RpcArguments(Map<String, Value>);

impl RpcArguments {
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

mod rpc_error {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fusen/src/rpc/error.rs"
    ));
}

pub use rpc_error::{ErrorCode, InvalidErrorCode, RetryHint, RpcCategory, RpcError, RpcOrigin};

mod rpc {
    pub(crate) use crate::rpc_error::ProblemDetails;
}

mod runtime {
    pub(crate) mod budget {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fusen/src/runtime/budget.rs"
        ));
    }

    pub(crate) mod deadline {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fusen/src/runtime/deadline.rs"
        ));
    }
}

pub struct RpcBody {
    bytes: Bytes,
    permit: Option<Arc<runtime::budget::BytePermit>>,
}

impl RpcBody {
    fn from_bytes(bytes: Bytes) -> Self {
        Self {
            bytes,
            permit: None,
        }
    }

    fn into_parts(self) -> (Bytes, Option<Arc<runtime::budget::BytePermit>>) {
        (self.bytes, self.permit)
    }

    fn hold_budget(&mut self, permit: runtime::budget::BytePermit) {
        self.permit = Some(Arc::new(permit));
    }
}

pub struct RpcResponse<T> {
    body: T,
    status: StatusCode,
    headers: HeaderMap,
}

impl<T> RpcResponse<T> {
    fn new(body: T) -> Self {
        Self {
            body,
            status: StatusCode::OK,
            headers: HeaderMap::new(),
        }
    }

    pub(crate) const fn status(&self) -> StatusCode {
        self.status
    }

    pub(crate) fn set_status(&mut self, status: StatusCode) -> Result<(), RpcError> {
        if !status.is_success() {
            return Err(RpcError::framework(
                RpcCategory::InvalidArgument,
                "invalid_response_status",
                "RPC success response status must be 2xx",
            ));
        }
        self.status = status;
        Ok(())
    }

    pub(crate) const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub(crate) fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }
}

impl RpcResponse<RpcBody> {
    fn fixture(result: Bytes) -> Self {
        Self::new(RpcBody::from_bytes(result))
    }

    pub(crate) fn from_json_bytes(result: Bytes) -> Self {
        Self::fixture(result)
    }

    pub(crate) fn success_with_budget<T: serde::Serialize>(
        value: T,
        limit: usize,
        wire_overhead: usize,
        budget: &Arc<runtime::budget::ByteBudget>,
    ) -> Result<Self, RpcError> {
        use runtime::budget::{BudgetedWriteFailure, BudgetedWriter};

        let exhausted = || {
            RpcError::framework(
                RpcCategory::ResourceExhausted,
                "response_byte_budget_exhausted",
                "server response byte budget is exhausted",
            )
        };
        let mut writer =
            BudgetedWriter::new(limit, budget, wire_overhead).map_err(|_failure| exhausted())?;
        serde_json::to_writer(&mut writer, &value).map_err(|error| match writer.failure() {
            Some(BudgetedWriteFailure::BudgetExhausted) => exhausted(),
            Some(BudgetedWriteFailure::LimitExceeded) | None => RpcError::framework(
                RpcCategory::Internal,
                "response_too_large",
                error.to_string(),
            ),
        })?;
        let (result, permit) = writer.into_parts();
        Ok(Self::new(RpcBody {
            bytes: result,
            permit: Some(permit),
        }))
    }

    pub(crate) fn into_wire_parts(
        self,
    ) -> (
        StatusCode,
        HeaderMap,
        Bytes,
        Option<Arc<runtime::budget::BytePermit>>,
    ) {
        let (bytes, permit) = self.body.into_parts();
        (self.status, self.headers, bytes, permit)
    }

    pub(crate) fn hold_budget(&mut self, permit: runtime::budget::BytePermit) {
        self.body.hold_budget(permit);
    }

    pub(crate) fn mark_declared_deserialize_schema_origin(
        &mut self,
        _method: &'static fusen_contract::MethodDescriptor,
    ) {
    }
}

mod middleware {
    pub(crate) trait MiddlewareDyn: Send + Sync {}
}

#[cfg(not(test))]
pub(crate) use middleware::MiddlewareDyn as Middleware;

mod service {
    pub(crate) trait ErasedDispatch: Send + Sync {}
}

#[allow(clippy::items_after_test_module)]
mod wire {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fusen/src/wire/mod.rs"
    ));

    pub(super) fn exercise_problem_details(data: &[u8]) {
        if let Ok(problem) = serde_json::from_slice::<ProblemDetails>(data) {
            let error = RpcError::from_remote(problem);
            let normalized = error.problem_details("fuzz-request", Some("/fuzz".to_owned()));
            let encoded = bounded_problem(&normalized);
            assert!(encoded.len() <= EMERGENCY_PROBLEM_LIMIT);
            let _: ProblemDetails = serde_json::from_slice(&encoded)
                .expect("the emergency Problem Details encoder must emit valid JSON");
        }

        let detail = String::from_utf8_lossy(data);
        if let Ok(error) =
            RpcError::application(http::StatusCode::BAD_REQUEST, "fuzz_application", detail)
        {
            let encoded = bounded_problem(&error.problem_details("fuzz-request", None));
            assert!(encoded.len() <= EMERGENCY_PROBLEM_LIMIT);
            let _: ProblemDetails = serde_json::from_slice(&encoded)
                .expect("the emergency Problem Details encoder must emit valid JSON");
        }
    }

    pub(super) fn exercise_spring_path(path: &str, query: &str, body: &[u8]) {
        let (service, method) = crate::descriptor();
        let budget = ByteBudget::new(64 * 1024);
        let mut arguments = RpcArguments::new();
        arguments.insert(
            "path".to_owned(),
            serde_json::Value::String(path.to_owned()),
        );
        arguments.insert(
            "query".to_owned(),
            serde_json::Value::String(query.to_owned()),
        );
        arguments.insert(
            "body".to_owned(),
            serde_json::from_slice(body).unwrap_or(serde_json::Value::Null),
        );
        if let Ok(template) = encode_request_template(
            service,
            method,
            WireProtocol::SpringCloudV1,
            &arguments,
            &HeaderMap::new(),
            64 * 1024,
            &budget,
        ) {
            let endpoint = "http://127.0.0.1:8080"
                .parse::<ServiceEndpoint>()
                .expect("static plaintext endpoint is valid");
            let _ = template.to_request(&endpoint, "fuzz-request", Duration::from_secs(1), 1);
        }
    }
}

#[allow(clippy::items_after_test_module)]
#[cfg(not(test))]
mod routes {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fusen/src/server/routes.rs"
    ));

    pub(super) fn exercise_path_and_query(path: &str, query: &str) {
        let _ = split_path(path);
        let max_pairs = query.len() % 256;
        let _ = validate_query_pairs(Some(query), max_pairs);
    }
}

fn descriptor() -> (
    &'static fusen_contract::ServiceDescriptor,
    &'static fusen_contract::MethodDescriptor,
) {
    use fusen_contract::{
        MethodDescriptor, MethodId, ServiceDescriptor, ServiceSelector, SpringCloudMethod,
        SpringCloudParameter, SpringCloudParameterCardinality, SpringCloudParameterSource,
    };

    static SERVICE: LazyLock<ServiceDescriptor> = LazyLock::new(|| {
        let parameters = vec![
            SpringCloudParameter::new(
                "path",
                SpringCloudParameterSource::Path,
                SpringCloudParameterCardinality::Scalar,
            )
            .unwrap(),
            SpringCloudParameter::new(
                "query",
                SpringCloudParameterSource::Query,
                SpringCloudParameterCardinality::Scalar,
            )
            .unwrap(),
            SpringCloudParameter::new(
                "body",
                SpringCloudParameterSource::Body,
                SpringCloudParameterCardinality::Scalar,
            )
            .unwrap(),
        ];
        let spring =
            SpringCloudMethod::new(http::Method::POST, "/fuzz/{path}", parameters).unwrap();
        let method = MethodDescriptor::new(MethodId::new(0), "fuzz", Some(spring)).unwrap();
        ServiceDescriptor::new(
            ServiceSelector::new("fuzz", None, None).unwrap(),
            vec![method],
        )
        .unwrap()
    });
    (&SERVICE, &SERVICE.methods()[0])
}

pub fn fuzz_wire_codec(data: &[u8]) {
    let max_body = data
        .first()
        .map_or(1024, |value| usize::from(*value).saturating_mul(256));
    let body = data.get(1..).unwrap_or_default();
    let decoded = wire::decode_fusen_request(body);
    if let Ok(arguments) = decoded {
        let (service, method) = descriptor();
        let request_budget = runtime::budget::ByteBudget::new(max_body.max(1));
        for protocol in [
            fusen_contract::WireProtocol::FusenV1,
            fusen_contract::WireProtocol::SpringCloudV1,
        ] {
            let _ = wire::encode_request_template(
                service,
                method,
                protocol,
                &arguments,
                &HeaderMap::new(),
                max_body,
                &request_budget,
            );
        }
    }

    let budget = runtime::budget::ByteBudget::new(max_body.max(1));
    for protocol in [
        fusen_contract::WireProtocol::FusenV1,
        fusen_contract::WireProtocol::SpringCloudV1,
    ] {
        let _ = wire::encode_success(
            protocol,
            RpcResponse::fixture(Bytes::copy_from_slice(body)),
            max_body,
            &budget,
        );
    }
}

pub fn fuzz_spring_path(data: &[u8]) {
    let separator = data
        .iter()
        .position(|byte| matches!(*byte, 0 | b'\n'))
        .unwrap_or(data.len());
    let path = String::from_utf8_lossy(&data[..separator]);
    let remainder = data.get(separator.saturating_add(1)..).unwrap_or_default();
    let query_separator = remainder
        .iter()
        .position(|byte| matches!(*byte, 0 | b'\n'))
        .unwrap_or(remainder.len());
    let query = String::from_utf8_lossy(&remainder[..query_separator]);
    let body = remainder
        .get(query_separator.saturating_add(1)..)
        .unwrap_or_default();
    routes::exercise_path_and_query(&path, &query);
    wire::exercise_spring_path(&path, &query, body);
}

pub fn fuzz_problem_details(data: &[u8]) {
    wire::exercise_problem_details(data);
}

#[cfg(test)]
mod routes {
    pub(super) fn exercise_path_and_query(_path: &str, _query: &str) {}
}
