#![allow(clippy::too_many_arguments, dead_code, missing_docs)]
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
pub struct Arguments(Map<String, Value>);

impl Arguments {
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

mod invocation_error {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fusen/src/error/invocation.rs"
    ));
}

pub(crate) use invocation_error::RemoteErrorParts;
pub use invocation_error::{
    Error, ErrorCategory, ErrorCode, ErrorConstructionError, ErrorDetails, ErrorKind, ErrorOrigin,
    InvalidErrorCode, RetryHint,
};

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

pub struct Body {
    bytes: Bytes,
    permit: Option<Arc<runtime::budget::BytePermit>>,
}

impl Body {
    fn from_bytes(bytes: Bytes) -> Self {
        Self {
            bytes,
            permit: None,
        }
    }

    fn len(&self) -> usize {
        self.bytes.len()
    }

    fn into_parts(self) -> (Bytes, Option<Arc<runtime::budget::BytePermit>>) {
        (self.bytes, self.permit)
    }

    fn hold_budget(&mut self, permit: runtime::budget::BytePermit) {
        self.permit = Some(Arc::new(permit));
    }
}

pub struct Response<T> {
    body: T,
    status: StatusCode,
    headers: HeaderMap,
}

impl<T> Response<T> {
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

    pub(crate) const fn body(&self) -> &T {
        &self.body
    }

    pub(crate) fn set_status(&mut self, status: StatusCode) -> Result<(), Error> {
        if !status.is_success() {
            return Err(Error::framework(
                ErrorCategory::InvalidArgument,
                "invalid_response_status",
                "service invocation success response status must be 2xx",
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

impl Response<Body> {
    fn fixture(result: Bytes) -> Self {
        Self::new(Body::from_bytes(result))
    }

    pub(crate) fn from_json_bytes(result: Bytes) -> Self {
        Self::fixture(result)
    }

    pub(crate) fn result_bytes(&self) -> &Bytes {
        &self.body.bytes
    }

    pub(crate) fn success_with_budget<T: serde::Serialize>(
        value: T,
        limit: usize,
        wire_overhead: usize,
        budget: &Arc<runtime::budget::ByteBudget>,
    ) -> Result<Self, Error> {
        use runtime::budget::{BudgetedWriteFailure, BudgetedWriter};

        let exhausted = || {
            Error::framework(
                ErrorCategory::ResourceExhausted,
                "response_byte_budget_exhausted",
                "server response byte budget is exhausted",
            )
        };
        let mut writer =
            BudgetedWriter::new(limit, budget, wire_overhead).map_err(|_failure| exhausted())?;
        serde_json::to_writer(&mut writer, &value).map_err(|error| match writer.failure() {
            Some(BudgetedWriteFailure::BudgetExhausted) => exhausted(),
            Some(BudgetedWriteFailure::LimitExceeded) | None => Error::framework(
                ErrorCategory::Internal,
                "response_too_large",
                error.to_string(),
            ),
        })?;
        let (result, permit) = writer.into_parts();
        Ok(Self::new(Body {
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

mod interceptor {
    pub(crate) trait InterceptorDyn: Send + Sync {}
}

#[cfg(not(test))]
pub(crate) use interceptor::InterceptorDyn as Interceptor;

mod service {
    pub(crate) trait ErasedDispatch: Send + Sync {}
}

mod codec {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fusen/src/codec.rs"
    ));
}

pub(crate) use codec::{
    BufferedResponse, EncodedRequest, ErrorDecoder, RequestEncoder, RequestEncoding,
    ResponseDecoder,
};

#[allow(clippy::items_after_test_module)]
mod wire {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fusen/src/wire/mod.rs"
    ));

    pub(super) fn exercise_problem_details(data: &[u8]) {
        let parsed = serde_json::from_slice::<problem::ProblemDetails>(data).ok();
        let status = parsed
            .as_ref()
            .and_then(|problem| http::StatusCode::from_u16(problem.status()).ok())
            .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
        let request_id = parsed
            .as_ref()
            .map(problem::ProblemDetails::request_id)
            .filter(|request_id| request_id_is_valid(request_id))
            .unwrap_or("fuzz-request");
        let mismatched_status = if status == http::StatusCode::BAD_GATEWAY {
            http::StatusCode::BAD_REQUEST
        } else {
            http::StatusCode::BAD_GATEWAY
        };
        let mismatched_request_id = if request_id == "mismatched-request" {
            "other-request"
        } else {
            "mismatched-request"
        };

        for strict_controls in [false, true] {
            for (response_status, expected_request_id) in [
                (status, request_id),
                (mismatched_status, request_id),
                (status, mismatched_request_id),
            ] {
                let error = problem::decode_problem(
                    response_status,
                    expected_request_id,
                    data,
                    HeaderMap::new(),
                    strict_controls,
                );
                let (normalized, _) =
                    problem::problem_from_error(&error, "fuzz-request", Some("/fuzz".to_owned()));
                let encoded = problem::bounded_problem(&normalized);
                assert!(encoded.len() <= EMERGENCY_PROBLEM_LIMIT);
                let _: problem::ProblemDetails = serde_json::from_slice(&encoded)
                    .expect("the emergency Problem Details encoder must emit valid JSON");
            }
        }

        let detail = String::from_utf8_lossy(data);
        if let Ok(error) =
            Error::application(ErrorCategory::InvalidArgument, "fuzz_application", detail)
        {
            let response = encode_problem(&error, "fuzz-request", None, true);
            assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);
            let (problem, _) = problem::problem_from_error(&error, "fuzz-request", None);
            let encoded = problem::bounded_problem(&problem);
            assert!(encoded.len() <= EMERGENCY_PROBLEM_LIMIT);
            let _: problem::ProblemDetails = serde_json::from_slice(&encoded)
                .expect("the emergency Problem Details encoder must emit valid JSON");
        }
    }

    pub(super) fn exercise_http_binding(path: &str, query: &str, body: &[u8]) {
        let (service, method) = crate::descriptor();
        let budget = ByteBudget::new(64 * 1024);
        let mut arguments = Arguments::new();
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
            &JsonCodec,
            service,
            method,
            &arguments,
            &HeaderMap::new(),
            64 * 1024,
            &budget,
        ) {
            let endpoint = "http://127.0.0.1:8080"
                .parse::<ServiceEndpoint>()
                .expect("static plaintext endpoint is valid");
            for (version, invocation_controls) in
                [(Version::HTTP_11, false), (Version::HTTP_2, true)]
            {
                let _ = template.to_request(
                    &endpoint,
                    version,
                    "fuzz-request",
                    Duration::from_secs(1),
                    1,
                    invocation_controls,
                    service,
                );
            }
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
        HttpOperation, HttpParameter, HttpParameterCardinality, HttpParameterSource,
        MethodDescriptor, MethodId, ServiceDescriptor, ServiceSelector,
    };

    static SERVICE: LazyLock<ServiceDescriptor> = LazyLock::new(|| {
        let parameters = vec![
            HttpParameter::new(
                "path",
                HttpParameterSource::Path,
                HttpParameterCardinality::Scalar,
            )
            .unwrap(),
            HttpParameter::new(
                "query",
                HttpParameterSource::Query,
                HttpParameterCardinality::Scalar,
            )
            .unwrap(),
            HttpParameter::new(
                "body",
                HttpParameterSource::Body,
                HttpParameterCardinality::Scalar,
            )
            .unwrap(),
        ];
        let operation = HttpOperation::new(
            http::Method::POST,
            "/fuzz/{path}",
            parameters,
            "application/json",
            "application/json",
        )
        .unwrap();
        let method = MethodDescriptor::new(MethodId::new(0), "fuzz", operation).unwrap();
        ServiceDescriptor::new(
            ServiceSelector::new("fuzz", None, None).unwrap(),
            vec![method],
        )
        .unwrap()
    });
    (&SERVICE, &SERVICE.methods()[0])
}

pub fn fuzz_wire_codec(data: &[u8]) {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum CorpusRequest {
        LegacyEnvelope { arguments: Arguments },
        Arguments(Arguments),
    }

    let max_body = data
        .first()
        .map_or(1024, |value| usize::from(*value).saturating_mul(256));
    let body = data.get(1..).unwrap_or_default();
    let decoded = serde_json::from_slice::<CorpusRequest>(body).map(|request| match request {
        CorpusRequest::LegacyEnvelope { arguments } | CorpusRequest::Arguments(arguments) => {
            arguments
        }
    });
    if let Ok(arguments) = decoded {
        let (service, method) = descriptor();
        let request_budget = runtime::budget::ByteBudget::new(max_body.max(1));
        let _ = wire::encode_request_template(
            &wire::JsonCodec,
            service,
            method,
            &arguments,
            &HeaderMap::new(),
            max_body,
            &request_budget,
        );
    }

    let budget = runtime::budget::ByteBudget::new(max_body.max(1));
    for suppress_body in [false, true] {
        let _ = wire::encode_success(
            Response::fixture(Bytes::copy_from_slice(body)),
            "application/json",
            suppress_body,
            max_body,
            &budget,
        );
    }
}

pub fn fuzz_http_binding(data: &[u8]) {
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
    wire::exercise_http_binding(&path, &query, body);
}

pub fn fuzz_problem_details(data: &[u8]) {
    wire::exercise_problem_details(data);
}

#[cfg(test)]
mod corpus_tests {
    use super::*;

    #[test]
    fn arbitrary_bytes_reach_the_problem_decoder() {
        fuzz_problem_details(b"\xff\0not-json%GG");
    }

    #[test]
    fn existing_problem_corpus_exercises_the_decoder_matrix() {
        fuzz_problem_details(include_bytes!(
            "../../fuzz/corpus/problem_details/application.json"
        ));
    }

    #[test]
    fn existing_wire_corpus_exercises_the_http_json_codec() {
        fuzz_wire_codec(include_bytes!(
            "../../fuzz/corpus/wire_codec/fusen-request.json"
        ));
    }

    #[test]
    fn existing_http_binding_corpus_exercises_path_query_and_body_encoding() {
        fuzz_http_binding(include_bytes!(
            "../../fuzz/corpus/http_binding/path-query-body.txt"
        ));
    }
}

#[cfg(test)]
mod routes {
    pub(super) fn exercise_path_and_query(_path: &str, _query: &str) {}
}
