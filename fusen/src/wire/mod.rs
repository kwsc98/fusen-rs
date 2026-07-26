use crate::{
    Arguments, ProblemDetails, RpcCategory, RpcError, RpcResponse,
    runtime::{
        budget::{BudgetedWriteFailure, BudgetedWriter, ByteBudget, BytePermit},
        deadline::Deadline,
    },
};
use bytes::{Buf, Bytes};
use fusen_contract::{
    Idempotency, MethodDescriptor, ServiceDescriptor, ServiceEndpoint, SpringCloudParameterSource,
    WireProtocol,
};
use http::{
    HeaderMap, HeaderName, HeaderValue, Method, Request, Response, Uri, Version,
    header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, TE, TRAILER, TRANSFER_ENCODING, UPGRADE},
};
use hyper::body::{Body, Frame, Incoming};
use serde::{Deserialize, Serialize};
use serde_json::{Value, value::RawValue};
use std::{
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use uuid::Uuid;

pub(crate) const FUSEN_CONTENT_TYPE: &str = "application/fusen+json;version=1";
pub(crate) const JSON_CONTENT_TYPE: &str = "application/json";
pub(crate) const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";
pub(crate) const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
pub(crate) const TIMEOUT_MS: HeaderName = HeaderName::from_static("x-fusen-timeout-ms");
pub(crate) const ATTEMPT: HeaderName = HeaderName::from_static("x-fusen-attempt");
pub(crate) const SERVICE_GROUP: HeaderName = HeaderName::from_static("x-fusen-service-group");
pub(crate) const SERVICE_VERSION: HeaderName = HeaderName::from_static("x-fusen-service-version");
const MAX_TIMEOUT_MS: u64 = 86_400_000;
const EMERGENCY_PROBLEM_LIMIT: usize = 4 * 1024;
const CHUNK_RESERVATION: usize = 4 * 1024;

#[derive(Debug)]
pub(crate) struct RequestControl {
    pub request_id: String,
    pub deadline: Deadline,
    pub attempt: u8,
}

pub(crate) fn parse_request_control(
    headers: &HeaderMap,
    local_timeout: Duration,
) -> Result<RequestControl, RpcError> {
    let request_id = match one_header(headers, &REQUEST_ID)? {
        Some(value) => {
            let value = value.to_str().map_err(|_| invalid_request_id())?;
            validate_request_id(value)?;
            value.to_owned()
        }
        None => Uuid::new_v4().simple().to_string(),
    };
    let wire_timeout = match one_header(headers, &TIMEOUT_MS)? {
        Some(value) => {
            let value = value.to_str().map_err(|_| invalid_timeout())?;
            let millis = value.parse::<u64>().map_err(|_| invalid_timeout())?;
            if millis > MAX_TIMEOUT_MS {
                return Err(invalid_timeout());
            }
            Duration::from_millis(millis)
        }
        None => local_timeout,
    };
    let attempt = match one_header(headers, &ATTEMPT)? {
        Some(value) => value
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|attempt| *attempt > 0)
            .ok_or_else(invalid_attempt)?,
        None => 1,
    };
    Ok(RequestControl {
        request_id,
        deadline: Deadline::after(local_timeout.min(wire_timeout)),
        attempt,
    })
}

pub(crate) fn validate_attempt(attempt: u8, idempotency: Idempotency) -> Result<(), RpcError> {
    if attempt > 1 && !idempotency.is_idempotent() {
        Err(invalid_attempt())
    } else {
        Ok(())
    }
}

pub(crate) fn validate_request_id(value: &str) -> Result<(), RpcError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(invalid_request_id())
    }
}

fn one_header<'a>(
    headers: &'a HeaderMap,
    name: &HeaderName,
) -> Result<Option<&'a HeaderValue>, RpcError> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(RpcError::framework(
            RpcCategory::InvalidArgument,
            "duplicate_control_header",
            format!("control header {name} must appear at most once"),
        ));
    }
    Ok(value)
}

fn validate_response_content_type(
    headers: &HeaderMap,
    expected: &'static str,
) -> Result<(), RpcError> {
    let mut values = headers.get_all(CONTENT_TYPE).iter();
    let Some(value) = values.next() else {
        return Err(invalid_response_content_type(expected));
    };
    if values.next().is_some() || value.to_str().ok() != Some(expected) {
        return Err(invalid_response_content_type(expected));
    }
    Ok(())
}

fn invalid_response_content_type(expected: &'static str) -> RpcError {
    RpcError::framework(
        RpcCategory::DataLoss,
        "invalid_content_type",
        format!("response Content-Type must be exactly one {expected} value"),
    )
}

fn invalid_request_id() -> RpcError {
    RpcError::framework(
        RpcCategory::InvalidArgument,
        "invalid_request_id",
        "x-request-id must be 1-64 ASCII letters, digits, '.', '_' or '-'",
    )
}

fn invalid_timeout() -> RpcError {
    RpcError::framework(
        RpcCategory::InvalidArgument,
        "invalid_timeout",
        "x-fusen-timeout-ms must be an integer from 0 through 86400000",
    )
}

fn invalid_attempt() -> RpcError {
    RpcError::framework(
        RpcCategory::InvalidArgument,
        "invalid_attempt",
        "x-fusen-attempt must start at one and retries require an idempotent method",
    )
}

#[derive(Serialize)]
struct FusenRequest<'a> {
    arguments: &'a Arguments,
}

#[derive(Deserialize)]
struct FusenRequestOwned {
    arguments: Arguments,
}

#[derive(Deserialize)]
struct FusenSuccessRaw<'a> {
    #[serde(borrow)]
    result: &'a RawValue,
}

pub(crate) struct RequestTemplate {
    pub method: Method,
    pub version: Version,
    pub path: String,
    pub query: Option<String>,
    pub headers: HeaderMap,
    pub body: Bytes,
    budget_permit: Arc<BytePermit>,
}

impl RequestTemplate {
    pub(crate) fn to_request(
        &self,
        endpoint: &ServiceEndpoint,
        request_id: &str,
        remaining: Duration,
        attempt: u8,
    ) -> Result<Request<GuardedBody>, RpcError> {
        let uri = endpoint_uri(endpoint, &self.path, self.query.as_deref())?;
        let mut request = Request::builder()
            .method(self.method.clone())
            .version(self.version)
            .uri(uri)
            .body(GuardedBody::new(
                self.body.clone(),
                Some(self.budget_permit.clone()),
            ))
            .map_err(|error| RpcError::internal("failed to build HTTP request", error))?;
        *request.headers_mut() = self.headers.clone();
        request.headers_mut().insert(
            REQUEST_ID,
            HeaderValue::from_str(request_id)
                .map_err(|error| RpcError::internal("invalid generated request ID", error))?,
        );
        let timeout_ms = remaining.as_millis().min(MAX_TIMEOUT_MS as u128);
        request.headers_mut().insert(
            TIMEOUT_MS,
            HeaderValue::from_str(&timeout_ms.to_string())
                .map_err(|error| RpcError::internal("invalid timeout header", error))?,
        );
        request.headers_mut().insert(
            ATTEMPT,
            HeaderValue::from_str(&attempt.to_string())
                .map_err(|error| RpcError::internal("invalid attempt header", error))?,
        );
        Ok(request)
    }
}

pub(crate) fn encode_request_template(
    service: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    protocol: WireProtocol,
    arguments: &Arguments,
    application_headers: &HeaderMap,
    max_body: usize,
    budget: &Arc<ByteBudget>,
) -> Result<RequestTemplate, RpcError> {
    let mut headers = application_headers.clone();
    for name in [
        CONTENT_TYPE,
        CONTENT_LENGTH,
        REQUEST_ID,
        TIMEOUT_MS,
        ATTEMPT,
        SERVICE_GROUP,
        SERVICE_VERSION,
    ] {
        headers.remove(name);
    }
    match protocol {
        WireProtocol::FusenV1 => {
            let path = format!(
                "/_fusen/v1/{}/{}",
                service.selector().service_id(),
                method.fusen_identity()
            );
            let (body, budget_permit) = budgeted_json(
                &FusenRequest { arguments },
                max_body,
                budget,
                "request_too_large",
            )?;
            headers.insert(CONTENT_TYPE, HeaderValue::from_static(FUSEN_CONTENT_TYPE));
            if let Some(group) = service.selector().group() {
                headers.insert(
                    SERVICE_GROUP,
                    HeaderValue::from_str(group)
                        .map_err(|error| RpcError::internal("invalid service group", error))?,
                );
            }
            if let Some(version) = service.selector().version() {
                headers.insert(
                    SERVICE_VERSION,
                    HeaderValue::from_str(version)
                        .map_err(|error| RpcError::internal("invalid service version", error))?,
                );
            }
            Ok(RequestTemplate {
                method: Method::POST,
                version: Version::HTTP_2,
                path,
                query: None,
                headers,
                body,
                budget_permit,
            })
        }
        WireProtocol::SpringCloudV1 => {
            let spring = method.spring_cloud().ok_or_else(|| {
                RpcError::framework(
                    RpcCategory::Unimplemented,
                    "spring_mapping_missing",
                    "method has no SpringCloudV1 mapping",
                )
            })?;
            let mut path = spring.path().to_owned();
            let mut query = Vec::new();
            let mut body = None;
            for parameter in spring.parameters() {
                let value = arguments
                    .get(parameter.name())
                    .cloned()
                    .unwrap_or(Value::Null);
                match parameter.source() {
                    SpringCloudParameterSource::Path => {
                        let value = scalar_text(&value, parameter.name())?;
                        path = path.replace(
                            &format!("{{{}}}", parameter.name()),
                            &urlencoding::encode(&value),
                        );
                    }
                    SpringCloudParameterSource::Query => {
                        append_query(&mut query, parameter.name(), &value)?
                    }
                    SpringCloudParameterSource::Body => body = Some(value),
                    _ => return Err(unsupported_spring_parameter_source()),
                }
            }
            let query = (!query.is_empty()).then(|| query.join("&"));
            let (body, budget_permit) = match body {
                Some(value) => budgeted_json(&value, max_body, budget, "request_too_large")?,
                None => {
                    let permit = budget.try_reserve(0).ok_or_else(request_budget_exhausted)?;
                    (Bytes::new(), Arc::new(permit))
                }
            };
            if !body.is_empty() {
                headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
            }
            Ok(RequestTemplate {
                method: spring.method().clone(),
                version: Version::HTTP_11,
                path,
                query,
                headers,
                body,
                budget_permit,
            })
        }
        _ => Err(unsupported_wire_protocol()),
    }
}

fn append_query(query: &mut Vec<String>, name: &str, value: &Value) -> Result<(), RpcError> {
    match value {
        Value::Null => Ok(()),
        Value::Array(values) => {
            for value in values {
                let value = scalar_text(value, name)?;
                query.push(format!(
                    "{}={}",
                    urlencoding::encode(name),
                    urlencoding::encode(&value)
                ));
            }
            Ok(())
        }
        value => {
            let value = scalar_text(value, name)?;
            query.push(format!(
                "{}={}",
                urlencoding::encode(name),
                urlencoding::encode(&value)
            ));
            Ok(())
        }
    }
}

fn scalar_text(value: &Value, name: &str) -> Result<String, RpcError> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err(RpcError::framework(
            RpcCategory::InvalidArgument,
            "invalid_spring_parameter",
            format!("Spring path/query argument {name} must be a scalar"),
        )),
    }
}

fn endpoint_uri(
    endpoint: &ServiceEndpoint,
    path: &str,
    query: Option<&str>,
) -> Result<Uri, RpcError> {
    if endpoint.as_url().scheme() != "http" {
        return Err(RpcError::framework(
            RpcCategory::InvalidArgument,
            "https_not_supported",
            "core transport accepts only plaintext http endpoints",
        ));
    }
    let mut value = endpoint.as_str().trim_end_matches('/').to_owned();
    if !path.starts_with('/') {
        value.push('/');
    }
    value.push_str(path);
    if let Some(query) = query {
        value.push('?');
        value.push_str(query);
    }
    value.parse::<Uri>().map_err(|error| {
        RpcError::internal("failed to construct canonical plaintext HTTP URI", error)
    })
}

pub(crate) fn decode_fusen_request(bytes: &[u8]) -> Result<Arguments, RpcError> {
    serde_json::from_slice::<FusenRequestOwned>(bytes)
        .map(|request| request.arguments)
        .map_err(|_| {
            RpcError::framework(
                RpcCategory::InvalidArgument,
                "invalid_json",
                "request is not a valid FusenV1 JSON envelope",
            )
        })
}

pub(crate) fn encode_success(
    protocol: WireProtocol,
    response: RpcResponse,
    max_body: usize,
    budget: &Arc<ByteBudget>,
) -> Result<Response<GuardedBody>, RpcError> {
    let (status, headers, result, existing_permit) = response.into_wire_parts();
    let total = match protocol {
        WireProtocol::FusenV1 => {
            let total = result
                .len()
                .checked_add(11)
                .ok_or_else(response_too_large)?;
            if total > max_body {
                return Err(response_too_large());
            }
            total
        }
        WireProtocol::SpringCloudV1 => {
            if result.len() > max_body {
                return Err(response_too_large());
            }
            result.len()
        }
        _ => return Err(unsupported_wire_protocol()),
    };
    let permit = match existing_permit {
        Some(permit) if permit.belongs_to(budget) => {
            let missing = total.saturating_sub(permit.bytes());
            if missing > 0 && !permit.grow(missing) {
                return Err(response_budget_exhausted());
            }
            permit
        }
        _ => Arc::new(
            budget
                .try_reserve(total)
                .ok_or_else(response_budget_exhausted)?,
        ),
    };
    let body = match protocol {
        WireProtocol::FusenV1 => GuardedBody::from_chunks(
            Bytes::from_static(b"{\"result\":"),
            result,
            Bytes::from_static(b"}"),
            Some(permit),
        ),
        WireProtocol::SpringCloudV1 => GuardedBody::new(result, Some(permit)),
        _ => return Err(unsupported_wire_protocol()),
    };
    let mut encoded = Response::builder()
        .status(status)
        .body(body)
        .map_err(|error| RpcError::internal("failed to build HTTP response", error))?;
    *encoded.headers_mut() = response_headers_without_control(headers);
    encoded.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static(match protocol {
            WireProtocol::FusenV1 => FUSEN_CONTENT_TYPE,
            WireProtocol::SpringCloudV1 => JSON_CONTENT_TYPE,
            _ => return Err(unsupported_wire_protocol()),
        }),
    );
    Ok(encoded)
}

fn response_budget_exhausted() -> RpcError {
    RpcError::framework(
        RpcCategory::ResourceExhausted,
        "response_byte_budget_exhausted",
        "server response byte budget is exhausted",
    )
}

fn response_too_large() -> RpcError {
    RpcError::framework(
        RpcCategory::Internal,
        "response_too_large",
        "encoded RPC response exceeds the configured limit",
    )
}

pub(crate) fn encode_problem(
    error: &RpcError,
    request_id: &str,
    instance: Option<String>,
) -> Response<GuardedBody> {
    let problem = error.problem_details(request_id, instance);
    let body = bounded_problem(&problem);
    Response::builder()
        .status(error.status())
        .header(CONTENT_TYPE, PROBLEM_CONTENT_TYPE)
        .header(REQUEST_ID, request_id)
        .body(GuardedBody::new(body, None))
        .expect("static problem response is valid")
}

fn bounded_problem(problem: &ProblemDetails) -> Bytes {
    let mut writer = LimitedWriter::new(EMERGENCY_PROBLEM_LIMIT);
    match serde_json::to_writer(&mut writer, problem) {
        Ok(()) => Bytes::from(writer.into_inner()),
        _ => {
            let minimal = ProblemDetails {
                type_uri: problem.type_uri.clone(),
                title: problem.title.clone(),
                status: problem.status,
                detail: None,
                instance: None,
                code: problem.code.clone(),
                request_id: problem.request_id.clone(),
                retryable: problem.retryable,
            };
            let mut writer = LimitedWriter::new(EMERGENCY_PROBLEM_LIMIT);
            serde_json::to_writer(&mut writer, &minimal)
                .expect("validated problem metadata fits the emergency response budget");
            Bytes::from(writer.into_inner())
        }
    }
}

pub(crate) async fn decode_http_response(
    protocol: WireProtocol,
    spring_head: bool,
    response: Response<Incoming>,
    max_body: usize,
    budget: &std::sync::Arc<ByteBudget>,
) -> Result<RpcResponse, RpcError> {
    let status = response.status();
    let expected_content_type = if status.is_success() {
        match protocol {
            WireProtocol::FusenV1 => FUSEN_CONTENT_TYPE,
            WireProtocol::SpringCloudV1 => JSON_CONTENT_TYPE,
            _ => return Err(unsupported_wire_protocol()),
        }
    } else {
        PROBLEM_CONTENT_TYPE
    };
    validate_response_content_type(response.headers(), expected_content_type)?;
    if spring_head && protocol == WireProtocol::SpringCloudV1 {
        let (parts, body) = response.into_parts();
        drop(body);
        if !status.is_success() {
            return Err(RpcError::from_remote_head(status));
        }
        let permit = budget
            .try_reserve(0)
            .ok_or_else(response_budget_exhausted)?;
        let mut rpc = RpcResponse::from_json_bytes(Bytes::from_static(b"null"));
        rpc.hold_budget(permit);
        *rpc.headers_mut() = response_headers_without_control(parts.headers);
        rpc.set_status(status)?;
        return Ok(rpc);
    }
    let content_length = parse_content_length(response.headers())?;
    let (_parts, body) = response.into_parts();
    let (body, permit) = read_body(body, content_length, max_body, budget).await?;
    if !status.is_success() {
        let problem = serde_json::from_slice::<ProblemDetails>(&body).map_err(|_| {
            RpcError::framework(
                RpcCategory::DataLoss,
                "invalid_problem_details",
                "remote error body is not valid Problem Details JSON",
            )
        })?;
        validate_problem_status(status, &problem)?;
        return Err(RpcError::from_remote(problem));
    }
    let result = match protocol {
        WireProtocol::FusenV1 => {
            let envelope = serde_json::from_slice::<FusenSuccessRaw<'_>>(&body).map_err(|_| {
                RpcError::framework(
                    RpcCategory::DataLoss,
                    "invalid_json",
                    "FusenV1 success body is invalid",
                )
            })?;
            bytes_subslice(&body, envelope.result.get().as_bytes()).ok_or_else(|| {
                RpcError::framework(
                    RpcCategory::DataLoss,
                    "invalid_json",
                    "FusenV1 result is not backed by the response body",
                )
            })?
        }
        WireProtocol::SpringCloudV1 => body,
        _ => return Err(unsupported_wire_protocol()),
    };
    let mut rpc = RpcResponse::from_json_bytes(result);
    rpc.hold_budget(permit);
    *rpc.headers_mut() = response_headers_without_control(_parts.headers);
    rpc.set_status(status)?;
    Ok(rpc)
}

fn validate_problem_status(
    status: http::StatusCode,
    problem: &ProblemDetails,
) -> Result<(), RpcError> {
    if problem.status == status.as_u16() {
        Ok(())
    } else {
        Err(RpcError::framework(
            RpcCategory::DataLoss,
            "invalid_problem_status",
            "Problem Details status does not match the HTTP status",
        ))
    }
}

fn bytes_subslice(parent: &Bytes, child: &[u8]) -> Option<Bytes> {
    let parent_start = parent.as_ptr() as usize;
    let child_start = child.as_ptr() as usize;
    let offset = child_start.checked_sub(parent_start)?;
    let end = offset.checked_add(child.len())?;
    (end <= parent.len()).then(|| parent.slice(offset..end))
}

fn response_headers_without_control(mut headers: HeaderMap) -> HeaderMap {
    for name in [
        CONNECTION,
        CONTENT_TYPE,
        CONTENT_LENGTH,
        TE,
        TRAILER,
        TRANSFER_ENCODING,
        UPGRADE,
        REQUEST_ID,
        TIMEOUT_MS,
        ATTEMPT,
        SERVICE_GROUP,
        SERVICE_VERSION,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
    headers
}

pub(crate) fn validate_protocol_version(
    protocol: WireProtocol,
    version: Version,
) -> Result<(), RpcError> {
    let valid = match protocol {
        WireProtocol::FusenV1 => version == Version::HTTP_2,
        WireProtocol::SpringCloudV1 => version == Version::HTTP_11,
        _ => return Err(unsupported_wire_protocol()),
    };
    if valid {
        Ok(())
    } else {
        Err(RpcError::framework(
            RpcCategory::InvalidArgument,
            "invalid_http_version",
            match protocol {
                WireProtocol::FusenV1 => "FusenV1 requires HTTP/2 prior knowledge",
                WireProtocol::SpringCloudV1 => "SpringCloudV1 requires HTTP/1.1",
                _ => "unsupported wire protocol",
            },
        ))
    }
}

fn unsupported_wire_protocol() -> RpcError {
    RpcError::framework(
        RpcCategory::Unimplemented,
        "unsupported_wire_protocol",
        "wire protocol is not supported by this runtime",
    )
}

fn unsupported_spring_parameter_source() -> RpcError {
    RpcError::framework(
        RpcCategory::Unimplemented,
        "unsupported_spring_parameter_source",
        "SpringCloudV1 parameter source is not supported",
    )
}

pub(crate) fn validate_content_type(
    headers: &HeaderMap,
    expected: &'static str,
    body_required: bool,
) -> Result<(), RpcError> {
    match one_header(headers, &CONTENT_TYPE)? {
        Some(value) if value.as_bytes() == expected.as_bytes() => Ok(()),
        None if !body_required => Ok(()),
        _ => Err(RpcError::framework(
            RpcCategory::InvalidArgument,
            "invalid_content_type",
            format!("request Content-Type must be {expected}"),
        )),
    }
}

pub(crate) fn parse_content_length(headers: &HeaderMap) -> Result<Option<usize>, RpcError> {
    let Some(value) = one_header(headers, &CONTENT_LENGTH)? else {
        return Ok(None);
    };
    value
        .to_str()
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(Some)
        .ok_or_else(|| {
            RpcError::framework(
                RpcCategory::InvalidArgument,
                "invalid_content_length",
                "Content-Length must be a non-negative integer",
            )
        })
}

pub(crate) async fn read_body(
    mut body: Incoming,
    content_length: Option<usize>,
    max_body: usize,
    budget: &std::sync::Arc<ByteBudget>,
) -> Result<(Bytes, BytePermit), RpcError> {
    if content_length.is_some_and(|length| length > max_body) {
        return Err(payload_too_large());
    }
    let initial_reservation = content_length.unwrap_or(0);
    let permit = budget.try_reserve(initial_reservation).ok_or_else(|| {
        RpcError::framework(
            RpcCategory::ResourceExhausted,
            "body_byte_budget_exhausted",
            "body byte budget is exhausted",
        )
    })?;
    let mut reserved = initial_reservation;
    let mut bytes = Vec::with_capacity(initial_reservation.min(16 * 1024));
    while let Some(frame) =
        std::future::poll_fn(|context| Pin::new(&mut body).poll_frame(context)).await
    {
        let frame = frame.map_err(|error| {
            RpcError::internal("HTTP body stream failed", error).mark_retryable()
        })?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        let next = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(payload_too_large)?;
        if next > max_body {
            return Err(payload_too_large());
        }
        if next > reserved {
            let needed = next - reserved;
            let growth = needed.div_ceil(CHUNK_RESERVATION) * CHUNK_RESERVATION;
            if !permit.grow(growth) {
                return Err(RpcError::framework(
                    RpcCategory::ResourceExhausted,
                    "body_byte_budget_exhausted",
                    "body byte budget is exhausted",
                ));
            }
            reserved += growth;
        }
        bytes.extend_from_slice(&chunk);
    }
    if content_length.is_some_and(|length| length != bytes.len()) {
        return Err(RpcError::framework(
            RpcCategory::InvalidArgument,
            "content_length_mismatch",
            "request body length does not match Content-Length",
        ));
    }
    Ok((Bytes::from(bytes), permit))
}

fn payload_too_large() -> RpcError {
    RpcError::framework(
        RpcCategory::PayloadTooLarge,
        "payload_too_large",
        "request or response body exceeds the configured limit",
    )
}

fn budgeted_json<T>(
    value: &T,
    max_body: usize,
    budget: &Arc<ByteBudget>,
    code: &'static str,
) -> Result<(Bytes, Arc<BytePermit>), RpcError>
where
    T: Serialize,
{
    let mut writer =
        BudgetedWriter::new(max_body, budget, 0).map_err(|_| request_budget_exhausted())?;
    serde_json::to_writer(&mut writer, value).map_err(|error| match writer.failure() {
        Some(BudgetedWriteFailure::LimitExceeded) => RpcError::framework(
            RpcCategory::PayloadTooLarge,
            code,
            "encoded request exceeds the configured limit",
        ),
        Some(BudgetedWriteFailure::BudgetExhausted) => request_budget_exhausted(),
        None => RpcError::internal("failed to encode JSON body", error),
    })?;
    Ok(writer.into_parts())
}

fn request_budget_exhausted() -> RpcError {
    RpcError::framework(
        RpcCategory::ResourceExhausted,
        "request_byte_budget_exhausted",
        "client request byte budget is exhausted",
    )
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl LimitedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(16 * 1024)),
            limit,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl std::io::Write for LimitedWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(buffer.len())
            .is_none_or(|length| length > self.limit)
        {
            return Err(std::io::Error::other("bounded JSON writer limit exceeded"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) struct GuardedBody {
    chunks: [Option<Bytes>; 3],
    next_chunk: usize,
    remaining: usize,
    permit: Option<Arc<BytePermit>>,
}

impl GuardedBody {
    pub(crate) fn new(body: Bytes, permit: Option<Arc<BytePermit>>) -> Self {
        let remaining = body.len();
        Self {
            chunks: [Some(body), None, None],
            next_chunk: 0,
            remaining,
            permit,
        }
    }

    fn from_chunks(
        prefix: Bytes,
        body: Bytes,
        suffix: Bytes,
        permit: Option<Arc<BytePermit>>,
    ) -> Self {
        let remaining = prefix
            .len()
            .checked_add(body.len())
            .and_then(|length| length.checked_add(suffix.len()))
            .expect("validated response body length fits usize");
        Self {
            chunks: [Some(prefix), Some(body), Some(suffix)],
            next_chunk: 0,
            remaining,
            permit,
        }
    }
}

#[derive(Debug)]
pub(crate) struct GuardedChunk {
    bytes: Bytes,
    _permit: Option<Arc<BytePermit>>,
}

impl Buf for GuardedChunk {
    fn remaining(&self) -> usize {
        self.bytes.remaining()
    }

    fn chunk(&self) -> &[u8] {
        self.bytes.chunk()
    }

    fn advance(&mut self, count: usize) {
        self.bytes.advance(count);
    }
}

impl Body for GuardedBody {
    type Data = GuardedChunk;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        while self.next_chunk < self.chunks.len() {
            let index = self.next_chunk;
            self.next_chunk += 1;
            let Some(chunk) = self.chunks[index].take() else {
                continue;
            };
            if chunk.is_empty() {
                continue;
            }
            self.remaining -= chunk.len();
            return Poll::Ready(Some(Ok(Frame::data(GuardedChunk {
                bytes: chunk,
                _permit: self.permit.clone(),
            }))));
        }
        Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        self.remaining == 0
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        hyper::body::SizeHint::with_exact(self.remaining as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusen_contract::Idempotency;
    use http::StatusCode;
    use http_body_util::BodyExt;

    #[test]
    fn request_ids_are_strict_ascii_tokens() {
        for invalid in ["", "has space", "中文", &"x".repeat(65)] {
            assert!(validate_request_id(invalid).is_err());
        }
        assert!(validate_request_id("request_1.a-b").is_ok());
    }

    #[test]
    fn duplicate_control_headers_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.append(REQUEST_ID, HeaderValue::from_static("one"));
        headers.append(REQUEST_ID, HeaderValue::from_static("two"));
        assert_eq!(
            parse_request_control(&headers, Duration::from_secs(1))
                .unwrap_err()
                .code()
                .as_str(),
            "duplicate_control_header"
        );
    }

    #[test]
    fn response_content_type_must_be_one_valid_utf8_value() {
        let mut invalid_utf8 = HeaderMap::new();
        invalid_utf8.insert(CONTENT_TYPE, HeaderValue::from_bytes(b"\x80").unwrap());
        let mut duplicate = HeaderMap::new();
        duplicate.append(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
        duplicate.append(CONTENT_TYPE, HeaderValue::from_static("text/plain"));

        for headers in [HeaderMap::new(), invalid_utf8, duplicate] {
            let error = validate_response_content_type(&headers, JSON_CONTENT_TYPE).unwrap_err();
            assert_eq!(error.category(), RpcCategory::DataLoss);
            assert_eq!(error.code().as_str(), "invalid_content_type");
        }

        let headers =
            HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE))]);
        validate_response_content_type(&headers, JSON_CONTENT_TYPE).unwrap();
    }

    #[test]
    fn non_idempotent_retries_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(ATTEMPT, HeaderValue::from_static("2"));
        assert_eq!(
            validate_attempt(
                parse_request_control(&headers, Duration::from_secs(1))
                    .unwrap()
                    .attempt,
                Idempotency::None,
            )
            .unwrap_err()
            .code()
            .as_str(),
            "invalid_attempt"
        );
    }

    #[test]
    fn bounded_json_stops_at_the_configured_limit() {
        let budget = ByteBudget::new(1024);
        let error = budgeted_json(&"x".repeat(128), 16, &budget, "request_too_large").unwrap_err();
        assert_eq!(error.code().as_str(), "request_too_large");
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn request_serialization_releases_partial_budget_when_exhausted() {
        let budget = ByteBudget::new(8);
        let error =
            budgeted_json(&"x".repeat(128), 1024, &budget, "request_too_large").unwrap_err();
        assert_eq!(error.code().as_str(), "request_byte_budget_exhausted");
        assert_eq!(budget.used(), 0);
    }

    #[tokio::test]
    async fn fusen_response_budget_covers_serialization_and_segmented_body() {
        let budget = ByteBudget::new(15);
        let response = RpcResponse::success_with_budget("ok", 4, 11, &budget).unwrap();
        assert_eq!(budget.used(), 15);
        let response = encode_success(WireProtocol::FusenV1, response, 15, &budget).unwrap();
        assert_eq!(response.body().size_hint().exact(), Some(15));
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body, r#"{"result":"ok"}"#);
        assert_eq!(budget.used(), 0);
    }

    #[tokio::test]
    async fn extracted_frame_holds_response_budget_after_body_is_dropped() {
        let budget = ByteBudget::new(4);
        let permit = Arc::new(budget.try_reserve(4).unwrap());
        let mut body = GuardedBody::new(Bytes::from_static(b"body"), Some(permit));

        let frame = body.frame().await.unwrap().unwrap();
        drop(body);
        assert_eq!(budget.used(), 4);

        let chunk = frame.into_data().unwrap();
        assert_eq!(chunk.chunk(), b"body");
        drop(chunk);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn response_encoding_strips_framing_and_framework_control_headers() {
        let budget = ByteBudget::new(1024);
        let mut response = RpcResponse::success_with_budget("ok", 1024, 0, &budget).unwrap();
        response
            .headers_mut()
            .insert(CONTENT_LENGTH, HeaderValue::from_static("999"));
        response
            .headers_mut()
            .insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        response
            .headers_mut()
            .insert(REQUEST_ID, HeaderValue::from_static("forged"));
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));

        let encoded = encode_success(WireProtocol::SpringCloudV1, response, 1024, &budget).unwrap();
        assert!(encoded.headers().get(CONTENT_LENGTH).is_none());
        assert!(encoded.headers().get(TRANSFER_ENCODING).is_none());
        assert!(encoded.headers().get(REQUEST_ID).is_none());
        assert_eq!(
            encoded.headers().get(CONTENT_TYPE).unwrap(),
            JSON_CONTENT_TYPE
        );
    }

    #[test]
    fn problem_details_status_must_match_the_http_status() {
        let problem = RpcError::framework(RpcCategory::Conflict, "conflict", "conflict")
            .problem_details("request", None);
        validate_problem_status(StatusCode::CONFLICT, &problem).unwrap();
        let error = validate_problem_status(StatusCode::BAD_REQUEST, &problem).unwrap_err();
        assert_eq!(error.code().as_str(), "invalid_problem_status");
    }

    #[test]
    fn oversized_problem_details_use_the_bounded_emergency_document() {
        let error = RpcError::application(
            http::StatusCode::BAD_REQUEST,
            "application_error",
            "x".repeat(EMERGENCY_PROBLEM_LIMIT * 2),
        )
        .unwrap();
        let body = bounded_problem(&error.problem_details("request", None));
        assert!(body.len() <= EMERGENCY_PROBLEM_LIMIT);
        let problem: ProblemDetails = serde_json::from_slice(&body).unwrap();
        assert_eq!(problem.status, StatusCode::BAD_REQUEST.as_u16());
        assert_eq!(problem.code.as_str(), "application_error");
        assert_eq!(problem.request_id, "request");
        assert!(!problem.retryable);
    }
}
