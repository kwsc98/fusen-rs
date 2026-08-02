use crate::{
    Arguments, Body, BufferedResponse, EncodedRequest, Error, ErrorCategory, ErrorDecoder,
    ErrorOrigin, RequestEncoder, RequestEncoding, Response, ResponseDecoder, RetryHint,
    runtime::{
        budget::{ByteBudget, BytePermit},
        deadline::Deadline,
    },
};
use bytes::{Buf, Bytes};
use fusen_contract::{
    HttpParameterCardinality, HttpParameterSource, MethodDescriptor, ServiceDescriptor,
    ServiceEndpoint,
};
use http::{
    HeaderMap, HeaderName, HeaderValue, Method, Request, Response as HttpResponse, StatusCode, Uri,
    Version,
    header::{
        ACCEPT, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, COOKIE, HOST, TE, TRAILER,
        TRANSFER_ENCODING, UPGRADE,
    },
};
use hyper::body::{Body as HttpBody, Frame, Incoming};
use serde_json::{Map, Value};
use std::{
    collections::HashSet,
    convert::Infallible,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    sync::Arc,
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use uuid::Uuid;

pub(crate) mod problem;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use problem::ProblemDetails;
use problem::{decode_head_error, decode_problem, validate_response_request_id};
pub(crate) use problem::{encode_problem, remote_protocol_error};

#[cfg(test)]
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
) -> Result<RequestControl, Error> {
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

pub(crate) fn validate_attempt(attempt: u8, method_allows_retries: bool) -> Result<(), Error> {
    if attempt > 1 && !method_allows_retries {
        Err(invalid_attempt())
    } else {
        Ok(())
    }
}

pub(crate) fn validate_request_id(value: &str) -> Result<(), Error> {
    if request_id_is_valid(value) {
        Ok(())
    } else {
        Err(invalid_request_id())
    }
}

pub(crate) fn validated_request_id_header(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(&REQUEST_ID).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    value
        .to_str()
        .ok()
        .filter(|value| request_id_is_valid(value))
}

fn request_id_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn one_header<'a>(
    headers: &'a HeaderMap,
    name: &HeaderName,
) -> Result<Option<&'a HeaderValue>, Error> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "duplicate_control_header",
            format!("control header {name} must appear at most once"),
        ));
    }
    Ok(value)
}

fn invalid_response_content_type(_expected: &str, request_id: &str, headers: &HeaderMap) -> Error {
    remote_protocol_error(
        "invalid_content_type",
        "response Content-Type does not match the declared JSON media type",
        request_id,
    )
    .with_headers(response_headers_without_control(headers.clone()))
}

fn parse_response_content_length(
    headers: &HeaderMap,
    request_id: &str,
) -> Result<Option<usize>, Error> {
    let mut values = headers.get_all(CONTENT_LENGTH).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(invalid_response_content_length(request_id, headers));
    }
    value
        .to_str()
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(Some)
        .ok_or_else(|| invalid_response_content_length(request_id, headers))
}

fn invalid_response_content_length(request_id: &str, headers: &HeaderMap) -> Error {
    remote_protocol_error(
        "invalid_content_length",
        "response Content-Length must be exactly one non-negative integer",
        request_id,
    )
    .with_headers(response_headers_without_control(headers.clone()))
}

fn invalid_request_id() -> Error {
    Error::framework(
        ErrorCategory::InvalidArgument,
        "invalid_request_id",
        "x-request-id must be 1-64 ASCII letters, digits, '.', '_' or '-'",
    )
}

fn invalid_timeout() -> Error {
    Error::framework(
        ErrorCategory::InvalidArgument,
        "invalid_timeout",
        "x-fusen-timeout-ms must be an integer from 0 through 86400000",
    )
}

fn invalid_attempt() -> Error {
    Error::framework(
        ErrorCategory::InvalidArgument,
        "invalid_attempt",
        "x-fusen-attempt must start at one and retries require an HTTP method that permits replay",
    )
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct JsonCodec;

impl RequestEncoder for JsonCodec {
    fn encode(&self, request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
        encode_json_request(request)
    }
}

impl ResponseDecoder for JsonCodec {
    fn decode(
        &self,
        method: &'static MethodDescriptor,
        response: BufferedResponse,
    ) -> Result<Response<Body>, Error> {
        if !matches!(
            response.status(),
            StatusCode::NO_CONTENT | StatusCode::RESET_CONTENT
        ) {
            validate_json_response_content_type(
                response.headers(),
                method.http_operation().produces(),
                response.request_id(),
            )?;
        }
        let (status, _version, headers, body) = response.into_parts();
        let mut decoded = Response::from_json_bytes(body);
        decoded.mark_declared_deserialize_schema_origin(method);
        *decoded.headers_mut() = headers;
        decoded.set_status(status)?;
        Ok(decoded)
    }
}

impl ErrorDecoder for JsonCodec {
    fn decode(&self, method: &'static MethodDescriptor, response: BufferedResponse) -> Error {
        let status = response.status();
        let request_id = response.request_id().to_owned();
        let controls = response.invocation_controls();
        let (_status, _version, headers, body) = response.into_parts();
        if *method.http_operation().method() == Method::HEAD && body.is_empty() {
            decode_head_error(status, &request_id, headers)
        } else {
            decode_problem(status, &request_id, &body, headers, controls)
        }
    }
}

#[derive(Debug)]
pub(crate) struct RequestTemplate {
    pub method: Method,
    pub path_and_query: String,
    pub headers: HeaderMap,
    pub body: Bytes,
    budget_permit: Arc<BytePermit>,
}

impl RequestTemplate {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn to_request(
        &self,
        endpoint: &ServiceEndpoint,
        version: Version,
        request_id: &str,
        remaining: Duration,
        attempt: u8,
        invocation_controls: bool,
        service: &ServiceDescriptor,
    ) -> Result<Request<GuardedBody>, Error> {
        let uri = endpoint_uri(endpoint, &self.path_and_query)?;
        let mut request = Request::builder()
            .method(self.method.clone())
            .version(version)
            .uri(uri)
            .body(GuardedBody::new(
                self.body.clone(),
                Some(self.budget_permit.clone()),
            ))
            .map_err(|error| Error::internal("failed to build HTTP request", error))?;
        *request.headers_mut() = self.headers.clone();
        if invocation_controls {
            request.headers_mut().insert(
                REQUEST_ID,
                HeaderValue::from_str(request_id)
                    .map_err(|error| Error::internal("invalid generated request ID", error))?,
            );
            let timeout_ms = remaining.as_millis().min(MAX_TIMEOUT_MS as u128);
            request.headers_mut().insert(
                TIMEOUT_MS,
                HeaderValue::from_str(&timeout_ms.to_string())
                    .map_err(|error| Error::internal("invalid timeout header", error))?,
            );
            request.headers_mut().insert(
                ATTEMPT,
                HeaderValue::from_str(&attempt.to_string())
                    .map_err(|error| Error::internal("invalid attempt header", error))?,
            );
            if let Some(group) = service.selector().group() {
                request.headers_mut().insert(
                    SERVICE_GROUP,
                    HeaderValue::from_str(group)
                        .map_err(|error| Error::internal("invalid service group", error))?,
                );
            }
            if let Some(service_version) = service.selector().version() {
                request.headers_mut().insert(
                    SERVICE_VERSION,
                    HeaderValue::from_str(service_version)
                        .map_err(|error| Error::internal("invalid service version", error))?,
                );
            }
        }
        Ok(request)
    }
}

pub(crate) fn encode_request_template(
    encoder: &dyn RequestEncoder,
    service: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    arguments: &Arguments,
    application_headers: &HeaderMap,
    max_body: usize,
    budget: &Arc<ByteBudget>,
) -> Result<RequestTemplate, Error> {
    let encoded = catch_unwind(AssertUnwindSafe(|| {
        encoder.encode(RequestEncoding::new(
            service,
            method,
            arguments,
            application_headers,
        ))
    }))
    .map_err(|_| codec_panic("request encoder"))?
    .map_err(Error::with_local_origin)?;
    let (http_method, path_and_query, headers, body) = encoded.into_parts();
    validate_encoded_request(method, &http_method, &path_and_query, &headers)?;
    if body.len() > max_body {
        return Err(request_too_large());
    }
    let budget_permit = Arc::new(
        budget
            .try_reserve(body.len())
            .ok_or_else(request_budget_exhausted)?,
    );
    Ok(RequestTemplate {
        method: http_method,
        path_and_query,
        headers,
        body,
        budget_permit,
    })
}

fn encode_json_request(request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
    let mapping = request.method().http_operation();
    let mut headers = request.headers().clone();
    reject_binding_owned_headers(&headers)?;
    let explicit_query = mapping
        .parameters()
        .iter()
        .filter(|parameter| parameter.source() == HttpParameterSource::Query)
        .map(|parameter| parameter.name())
        .collect::<HashSet<_>>();
    let explicit_headers = mapping
        .parameters()
        .iter()
        .filter(|parameter| parameter.source() == HttpParameterSource::Header)
        .map(|parameter| bound_header_name(parameter.name()))
        .collect::<Result<HashSet<_>, _>>()?;
    for name in &explicit_headers {
        if headers.contains_key(name) {
            return Err(header_conflict(name.as_str()));
        }
    }
    if mapping
        .parameters()
        .iter()
        .any(|parameter| parameter.source() == HttpParameterSource::Cookie)
        && headers.contains_key(COOKIE)
    {
        return Err(header_conflict(COOKIE.as_str()));
    }
    let mut path = mapping.path().to_owned();
    let mut query = Vec::new();
    let mut body = None;
    let mut body_fields = Map::new();
    let mut cookies = Vec::new();
    for parameter in mapping.parameters() {
        let value = request
            .arguments()
            .get(parameter.name())
            .cloned()
            .unwrap_or(Value::Null);
        match parameter.source() {
            HttpParameterSource::Path => {
                let value = scalar_text(&value, parameter.name())?;
                path = path.replace(
                    &format!("{{{}}}", parameter.name()),
                    &urlencoding::encode(&value),
                );
            }
            HttpParameterSource::Query => append_query(
                &mut query,
                parameter.name(),
                parameter.cardinality(),
                &value,
            )?,
            HttpParameterSource::Header => {
                if value != Value::Null {
                    insert_bound_header(&mut headers, parameter.name(), &value)?;
                }
            }
            HttpParameterSource::Cookie => {
                if value != Value::Null {
                    cookies.push(format!(
                        "{}={}",
                        parameter.name(),
                        cookie_text(&value, parameter.name())?
                    ));
                }
            }
            HttpParameterSource::BodyField => {
                body_fields.insert(parameter.name().to_owned(), value);
            }
            HttpParameterSource::Body => body = Some(value),
            HttpParameterSource::QueryMap => append_query_map(&mut query, &value, &explicit_query)?,
            HttpParameterSource::HeaderMap => {
                append_header_map(&mut headers, &value, &explicit_headers)?
            }
            _ => return Err(unsupported_http_parameter_source()),
        }
    }
    if body.is_none() && !body_fields.is_empty() {
        body = Some(Value::Object(body_fields));
    }
    if !cookies.is_empty() {
        if headers.contains_key(COOKIE) {
            return Err(header_conflict(COOKIE.as_str()));
        }
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&cookies.join("; ")).map_err(|_| {
                Error::framework(
                    ErrorCategory::InvalidArgument,
                    "invalid_cookie_parameter",
                    "cookie argument cannot be represented as an HTTP Cookie header",
                )
            })?,
        );
    }
    let body = body
        .map(|value| {
            serde_json::to_vec(&value)
                .map(Bytes::from)
                .map_err(|error| Error::internal("failed to encode JSON request body", error))
        })
        .transpose()?
        .unwrap_or_default();
    if !body.is_empty() {
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_str(mapping.consumes()).map_err(|_| {
                Error::framework(
                    ErrorCategory::InvalidArgument,
                    "invalid_consumes_media_type",
                    "request media type cannot be represented as Content-Type",
                )
            })?,
        );
    }
    headers.insert(
        ACCEPT,
        HeaderValue::from_str(mapping.produces()).map_err(|_| {
            Error::framework(
                ErrorCategory::InvalidArgument,
                "invalid_produces_media_type",
                "response media type cannot be represented as Accept",
            )
        })?,
    );
    let path_and_query = if query.is_empty() {
        path
    } else {
        format!("{path}?{}", query.join("&"))
    };
    Ok(EncodedRequest::new(
        mapping.method().clone(),
        path_and_query,
        headers,
        body,
    ))
}

fn append_query(
    query: &mut Vec<String>,
    name: &str,
    cardinality: HttpParameterCardinality,
    value: &Value,
) -> Result<(), Error> {
    match cardinality {
        HttpParameterCardinality::Scalar => match value {
            Value::Null => Ok(()),
            Value::Array(_) => Err(invalid_query_cardinality(
                name,
                "is an array but is declared scalar; add `#[param(query, repeated)]`",
            )),
            value => {
                let value = scalar_text(value, name)?;
                query.push(format!(
                    "{}={}",
                    urlencoding::encode(name),
                    urlencoding::encode(&value)
                ));
                Ok(())
            }
        },
        HttpParameterCardinality::Repeated => {
            let Value::Array(values) = value else {
                return Err(invalid_query_cardinality(
                    name,
                    "is declared repeated but did not serialize as an array; serialize it as an array or remove `repeated`",
                ));
            };
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
        _ => Err(unsupported_http_parameter_source()),
    }
}

fn invalid_query_cardinality(name: &str, detail: &str) -> Error {
    Error::framework(
        ErrorCategory::InvalidArgument,
        "invalid_http_parameter",
        format!("HTTP query argument {name} {detail}"),
    )
}

fn scalar_text(value: &Value, name: &str) -> Result<String, Error> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "invalid_http_parameter",
            format!("HTTP text argument {name} must be a scalar"),
        )),
    }
}

fn cookie_text(value: &Value, name: &str) -> Result<String, Error> {
    let value = scalar_text(value, name)?;
    if value.bytes().all(|byte| {
        matches!(
            byte,
            b'!' | b'#'..=b'+' | b'-'..=b':' | b'<'..=b'[' | b']'..=b'~'
        )
    }) {
        Ok(value)
    } else {
        Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "invalid_cookie_parameter",
            format!("cookie argument {name} contains a character that is not allowed on the wire"),
        ))
    }
}

fn append_query_map(
    query: &mut Vec<String>,
    value: &Value,
    explicit: &HashSet<&str>,
) -> Result<(), Error> {
    let Value::Object(values) = value else {
        if value == &Value::Null {
            return Ok(());
        }
        return Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "invalid_query_map",
            "query_map argument must serialize as a JSON object",
        ));
    };
    for (name, value) in values {
        if value == &Value::Null || matches!(value, Value::Array(values) if values.is_empty()) {
            continue;
        }
        if explicit.contains(name.as_str()) || query_contains(query, name) {
            return Err(Error::framework(
                ErrorCategory::InvalidArgument,
                "duplicate_query_parameter",
                format!("query_map conflicts with query parameter {name}"),
            ));
        }
        match value {
            Value::Array(values) => {
                for value in values {
                    append_query(query, name, HttpParameterCardinality::Scalar, value)?;
                }
            }
            value => append_query(query, name, HttpParameterCardinality::Scalar, value)?,
        }
    }
    Ok(())
}

fn query_contains(query: &[String], name: &str) -> bool {
    let prefix = format!("{}=", urlencoding::encode(name));
    query.iter().any(|pair| pair.starts_with(&prefix))
}

fn insert_bound_header(headers: &mut HeaderMap, name: &str, value: &Value) -> Result<(), Error> {
    let name = bound_header_name(name)?;
    if headers.contains_key(&name) || is_runtime_owned_header(&name) {
        return Err(header_conflict(name.as_str()));
    }
    let value = scalar_text(value, name.as_str())?;
    headers.insert(
        name,
        HeaderValue::from_str(&value).map_err(|_| {
            Error::framework(
                ErrorCategory::InvalidArgument,
                "invalid_header_parameter",
                "header argument cannot be represented as an HTTP header value",
            )
        })?,
    );
    Ok(())
}

fn bound_header_name(name: &str) -> Result<HeaderName, Error> {
    HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
        Error::framework(
            ErrorCategory::InvalidArgument,
            "invalid_header_parameter",
            "header argument name is not a valid HTTP header name",
        )
    })
}

fn append_header_map(
    headers: &mut HeaderMap,
    value: &Value,
    explicit: &HashSet<HeaderName>,
) -> Result<(), Error> {
    let Value::Object(values) = value else {
        if value == &Value::Null {
            return Ok(());
        }
        return Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "invalid_header_map",
            "header_map argument must serialize as a JSON object",
        ));
    };
    for (name, value) in values {
        if value == &Value::Null || matches!(value, Value::Array(values) if values.is_empty()) {
            continue;
        }
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            Error::framework(
                ErrorCategory::InvalidArgument,
                "invalid_header_map",
                "header_map contains an invalid HTTP header name",
            )
        })?;
        if explicit.contains(&name) || headers.contains_key(&name) || is_runtime_owned_header(&name)
        {
            return Err(header_conflict(name.as_str()));
        }
        let values: Vec<&Value> = match value {
            Value::Array(values) => values.iter().collect(),
            value => vec![value],
        };
        for value in values {
            let value = scalar_text(value, name.as_str())?;
            headers.append(
                name.clone(),
                HeaderValue::from_str(&value).map_err(|_| {
                    Error::framework(
                        ErrorCategory::InvalidArgument,
                        "invalid_header_map",
                        "header_map contains an invalid HTTP header value",
                    )
                })?,
            );
        }
    }
    Ok(())
}

fn header_conflict(name: &str) -> Error {
    Error::framework(
        ErrorCategory::InvalidArgument,
        "header_binding_conflict",
        format!("HTTP header {name} has more than one owner"),
    )
}

fn reject_binding_owned_headers(headers: &HeaderMap) -> Result<(), Error> {
    for name in [
        &CONTENT_TYPE,
        &ACCEPT,
        &REQUEST_ID,
        &TIMEOUT_MS,
        &ATTEMPT,
        &SERVICE_GROUP,
        &SERVICE_VERSION,
        &CONTENT_LENGTH,
        &HOST,
        &CONNECTION,
        &TE,
        &TRAILER,
        &TRANSFER_ENCODING,
        &UPGRADE,
    ] {
        if headers.contains_key(name) {
            return Err(header_conflict(name.as_str()));
        }
    }
    Ok(())
}

fn is_runtime_owned_header(name: &HeaderName) -> bool {
    [
        &CONTENT_TYPE,
        &ACCEPT,
        &REQUEST_ID,
        &TIMEOUT_MS,
        &ATTEMPT,
        &SERVICE_GROUP,
        &SERVICE_VERSION,
        &CONTENT_LENGTH,
        &HOST,
        &CONNECTION,
        &TE,
        &TRAILER,
        &TRANSFER_ENCODING,
        &UPGRADE,
    ]
    .contains(&name)
}

fn validate_encoded_request(
    descriptor: &MethodDescriptor,
    encoded_method: &Method,
    path_and_query: &str,
    headers: &HeaderMap,
) -> Result<(), Error> {
    if encoded_method != descriptor.http_operation().method() {
        return Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "invalid_encoded_request_method",
            "request encoder method must match the canonical HTTP operation",
        ));
    }
    if !path_and_query.starts_with('/')
        || path_and_query.parse::<http::uri::PathAndQuery>().is_err()
    {
        return Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "invalid_encoded_request_uri",
            "request encoder must produce a valid absolute-path URI without authority",
        ));
    }
    for name in [
        &REQUEST_ID,
        &TIMEOUT_MS,
        &ATTEMPT,
        &SERVICE_GROUP,
        &SERVICE_VERSION,
        &CONTENT_LENGTH,
        &HOST,
        &CONNECTION,
        &TE,
        &TRAILER,
        &TRANSFER_ENCODING,
        &UPGRADE,
    ] {
        if headers.contains_key(name) {
            return Err(header_conflict(name.as_str()));
        }
    }
    Ok(())
}

fn endpoint_uri(endpoint: &ServiceEndpoint, path_and_query: &str) -> Result<Uri, Error> {
    if !matches!(endpoint.as_url().scheme(), "http" | "https") {
        return Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "unsupported_endpoint_scheme",
            "client transport accepts only http or https endpoints",
        ));
    }
    let mut value = endpoint.as_str().trim_end_matches('/').to_owned();
    if !path_and_query.starts_with('/') {
        value.push('/');
    }
    value.push_str(path_and_query);
    value
        .parse::<Uri>()
        .map_err(|error| Error::internal("failed to construct canonical HTTP URI", error))
}

pub(crate) fn encode_success(
    response: Response<Body>,
    produces: &str,
    suppress_body: bool,
    max_body: usize,
    budget: &Arc<ByteBudget>,
) -> Result<HttpResponse<GuardedBody>, Error> {
    let (status, headers, result, existing_permit) = response.into_wire_parts();
    let suppress_body =
        suppress_body || matches!(status, StatusCode::NO_CONTENT | StatusCode::RESET_CONTENT);
    let total = if suppress_body { 0 } else { result.len() };
    if total > max_body {
        return Err(response_too_large());
    }
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
    let body = GuardedBody::new(
        if suppress_body { Bytes::new() } else { result },
        Some(permit),
    );
    let mut encoded = HttpResponse::builder()
        .status(status)
        .body(body)
        .map_err(|error| Error::internal("failed to build HTTP response", error))?;
    *encoded.headers_mut() = response_headers_without_control(headers);
    if !suppress_body {
        encoded.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_str(produces).map_err(|_| {
                Error::framework(
                    ErrorCategory::Internal,
                    "invalid_produces_media_type",
                    "configured response media type is not a valid Content-Type",
                )
            })?,
        );
    }
    Ok(encoded)
}

fn response_budget_exhausted() -> Error {
    Error::framework(
        ErrorCategory::ResourceExhausted,
        "response_byte_budget_exhausted",
        "server response byte budget is exhausted",
    )
}

fn response_too_large() -> Error {
    Error::framework(
        ErrorCategory::Internal,
        "response_too_large",
        "encoded invocation response exceeds the configured limit",
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn decode_http_response(
    response_decoder: &dyn ResponseDecoder,
    error_decoder: &dyn ErrorDecoder,
    head: bool,
    method: &'static MethodDescriptor,
    expected_request_id: &str,
    response: HttpResponse<Incoming>,
    max_body: usize,
    budget: &std::sync::Arc<ByteBudget>,
    invocation_controls: bool,
) -> Result<Response<Body>, Error> {
    let status = response.status();
    let version = response.version();
    validate_response_http_version(version, expected_request_id, response.headers())?;
    validate_response_request_id(invocation_controls, response.headers(), expected_request_id)?;
    if head {
        let (parts, body) = response.into_parts();
        drop(body);
        let decoder_headers = response_headers_for_decoder(parts.headers);
        let application_headers = response_headers_without_control(decoder_headers.clone());
        if !status.is_success() {
            let buffered = BufferedResponse::new(
                status,
                version,
                decoder_headers,
                Bytes::new(),
                expected_request_id.to_owned(),
                invocation_controls,
            );
            let error = catch_unwind(AssertUnwindSafe(|| error_decoder.decode(method, buffered)))
                .map_err(|_| codec_panic("error decoder"))?;
            return Err(error
                .with_remote_origin()
                .with_request_id(expected_request_id)
                .with_headers(application_headers));
        }
        let permit = budget
            .try_reserve(0)
            .ok_or_else(client_response_budget_exhausted)?;
        let mut response = Response::from_json_bytes(Bytes::from_static(b"null"));
        response.mark_declared_deserialize_schema_origin(method);
        response.hold_budget(permit);
        *response.headers_mut() = application_headers;
        response.set_status(status)?;
        return Ok(response);
    }
    if status.is_success() && matches!(status, StatusCode::NO_CONTENT | StatusCode::RESET_CONTENT) {
        let (parts, body) = response.into_parts();
        drop(body);
        let decoder_headers = response_headers_for_decoder(parts.headers);
        let application_headers = response_headers_without_control(decoder_headers.clone());
        let buffered = BufferedResponse::new(
            status,
            version,
            decoder_headers,
            Bytes::from_static(b"null"),
            expected_request_id.to_owned(),
            invocation_controls,
        );
        let mut decoded = catch_unwind(AssertUnwindSafe(|| {
            response_decoder.decode(method, buffered)
        }))
        .map_err(|_| codec_panic("response decoder"))?
        .map_err(|error| error.with_remote_origin())?;
        let permit = budget
            .try_reserve(0)
            .ok_or_else(client_response_budget_exhausted)?;
        hold_decoded_response_budget(&mut decoded, permit, max_body)?;
        *decoded.headers_mut() = application_headers;
        return Ok(decoded);
    }
    let content_length = parse_response_content_length(response.headers(), expected_request_id)?;
    let (parts, body) = response.into_parts();
    let decoder_headers = response_headers_for_decoder(parts.headers);
    let application_headers = response_headers_without_control(decoder_headers.clone());
    let (body, permit) =
        read_response_body(body, content_length, max_body, budget, expected_request_id)
            .await
            .map_err(|error| {
                if error.origin() == ErrorOrigin::Remote {
                    error.with_headers(application_headers.clone())
                } else {
                    error
                }
            })?;
    let buffered = BufferedResponse::new(
        status,
        version,
        decoder_headers,
        body,
        expected_request_id.to_owned(),
        invocation_controls,
    );
    if !status.is_success() {
        let error = catch_unwind(AssertUnwindSafe(|| error_decoder.decode(method, buffered)))
            .map_err(|_| codec_panic("error decoder"))?;
        drop(permit);
        return Err(error
            .with_remote_origin()
            .with_request_id(expected_request_id)
            .with_headers(application_headers));
    }
    let mut decoded = catch_unwind(AssertUnwindSafe(|| {
        response_decoder.decode(method, buffered)
    }))
    .map_err(|_| codec_panic("response decoder"))?
    .map_err(|error| error.with_remote_origin())?;
    hold_decoded_response_budget(&mut decoded, permit, max_body)?;
    *decoded.headers_mut() = application_headers;
    Ok(decoded)
}

fn hold_decoded_response_budget(
    response: &mut Response<Body>,
    permit: BytePermit,
    max_body: usize,
) -> Result<(), Error> {
    let decoded_length = response.body().len();
    if decoded_length > max_body {
        return Err(client_response_too_large());
    }
    let missing = decoded_length.saturating_sub(permit.bytes());
    if missing > 0 && !permit.grow(missing) {
        return Err(client_response_budget_exhausted());
    }
    response.hold_budget(permit);
    Ok(())
}

fn response_headers_without_hop(mut headers: HeaderMap) -> HeaderMap {
    for name in [CONNECTION, TE, TRAILER, TRANSFER_ENCODING, UPGRADE] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
    headers
}

fn response_headers_for_decoder(mut headers: HeaderMap) -> HeaderMap {
    headers = response_headers_without_hop(headers);
    for name in [
        REQUEST_ID,
        TIMEOUT_MS,
        ATTEMPT,
        SERVICE_GROUP,
        SERVICE_VERSION,
    ] {
        headers.remove(name);
    }
    headers
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

pub(crate) fn validate_http_version(version: Version) -> Result<(), Error> {
    if matches!(version, Version::HTTP_11 | Version::HTTP_2) {
        Ok(())
    } else {
        Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "invalid_http_version",
            "service invocation requires HTTP/1.1 or HTTP/2",
        ))
    }
}

fn validate_response_http_version(
    version: Version,
    request_id: &str,
    headers: &HeaderMap,
) -> Result<(), Error> {
    if matches!(version, Version::HTTP_11 | Version::HTTP_2) {
        Ok(())
    } else {
        Err(remote_protocol_error(
            "invalid_http_version",
            "remote response must use HTTP/1.1 or HTTP/2",
            request_id,
        )
        .with_headers(response_headers_without_control(headers.clone())))
    }
}

fn unsupported_http_parameter_source() -> Error {
    Error::framework(
        ErrorCategory::Unimplemented,
        "unsupported_http_parameter_source",
        "HTTP parameter source is not supported",
    )
}

pub(crate) fn validate_content_type(
    headers: &HeaderMap,
    expected: &str,
    body_required: bool,
) -> Result<(), Error> {
    match one_header(headers, &CONTENT_TYPE)? {
        Some(value) if media_type_matches(value, expected) => Ok(()),
        None if !body_required => Ok(()),
        _ => Err(Error::framework(
            ErrorCategory::InvalidArgument,
            "invalid_content_type",
            format!(
                "request Content-Type must be a JSON application media type compatible with {expected}"
            ),
        )),
    }
}

fn validate_json_response_content_type(
    headers: &HeaderMap,
    expected: &str,
    request_id: &str,
) -> Result<(), Error> {
    let mut values = headers.get_all(CONTENT_TYPE).iter();
    let Some(value) = values.next() else {
        return Err(invalid_response_content_type(expected, request_id, headers));
    };
    if values.next().is_some() || !media_type_matches(value, expected) {
        return Err(invalid_response_content_type(expected, request_id, headers));
    }
    Ok(())
}

pub(crate) fn validate_json_service(service: &ServiceDescriptor) -> Result<(), String> {
    for method in service.methods() {
        let operation = method.http_operation();
        for (field, value) in [
            ("consumes", operation.consumes()),
            ("produces", operation.produces()),
        ] {
            if !is_json_media_type(value) {
                return Err(format!(
                    "method {} has {field} media type {value:?}; http-json-v1 requires application/json or a concrete application subtype ending in +json",
                    method.invocation_name(),
                ));
            }
        }
    }
    Ok(())
}

fn media_type_matches(value: &HeaderValue, expected: &str) -> bool {
    let Ok(actual) = value
        .to_str()
        .ok()
        .unwrap_or_default()
        .parse::<mime::Mime>()
    else {
        return false;
    };
    let Ok(expected) = expected.parse::<mime::Mime>() else {
        return false;
    };
    is_json_mime(&actual) && is_json_mime(&expected)
}

fn is_json_media_type(value: &str) -> bool {
    value
        .parse::<mime::Mime>()
        .is_ok_and(|value| is_json_mime(&value))
}

fn is_json_mime(value: &mime::Mime) -> bool {
    value.type_() == mime::APPLICATION
        && value.subtype() != mime::STAR
        && match value.suffix() {
            Some(suffix) => suffix == mime::JSON,
            None => value.subtype() == mime::JSON,
        }
}

fn codec_panic(component: &str) -> Error {
    tracing::error!(component, "HTTP codec panicked");
    Error::framework(
        ErrorCategory::Internal,
        "codec_panic",
        "HTTP codec panicked while processing a service invocation",
    )
}

fn request_too_large() -> Error {
    Error::framework(
        ErrorCategory::PayloadTooLarge,
        "request_too_large",
        "encoded request body exceeds the configured limit",
    )
}

pub(crate) fn parse_content_length(headers: &HeaderMap) -> Result<Option<usize>, Error> {
    let Some(value) = one_header(headers, &CONTENT_LENGTH)? else {
        return Ok(None);
    };
    value
        .to_str()
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(Some)
        .ok_or_else(|| {
            Error::framework(
                ErrorCategory::InvalidArgument,
                "invalid_content_length",
                "Content-Length must be a non-negative integer",
            )
        })
}

pub(crate) async fn read_body(
    body: Incoming,
    content_length: Option<usize>,
    max_body: usize,
    budget: &std::sync::Arc<ByteBudget>,
) -> Result<(Bytes, BytePermit), Error> {
    read_body_with_role(
        body,
        content_length,
        max_body,
        budget,
        BodyReadRole::Request,
    )
    .await
}

async fn read_response_body(
    body: Incoming,
    content_length: Option<usize>,
    max_body: usize,
    budget: &std::sync::Arc<ByteBudget>,
    request_id: &str,
) -> Result<(Bytes, BytePermit), Error> {
    read_body_with_role(
        body,
        content_length,
        max_body,
        budget,
        BodyReadRole::Response { request_id },
    )
    .await
}

#[derive(Clone, Copy)]
enum BodyReadRole<'a> {
    Request,
    Response { request_id: &'a str },
}

impl BodyReadRole<'_> {
    fn too_large(self) -> Error {
        match self {
            Self::Request => payload_too_large(),
            Self::Response { .. } => client_response_too_large(),
        }
    }

    fn budget_exhausted(self) -> Error {
        match self {
            Self::Request => Error::framework(
                ErrorCategory::ResourceExhausted,
                "body_byte_budget_exhausted",
                "body byte budget is exhausted",
            ),
            Self::Response { .. } => client_response_budget_exhausted(),
        }
    }

    fn stream_failed<E>(self, error: E) -> Error
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        match self {
            Self::Request => Error::internal("HTTP body stream failed", error)
                .with_retry_hint(RetryHint::Retryable),
            Self::Response { request_id } => remote_protocol_error(
                "response_body_stream_failed",
                "remote response body stream failed before completion",
                request_id,
            )
            .with_source(error),
        }
    }

    fn content_length_mismatch(self) -> Error {
        match self {
            Self::Request => Error::framework(
                ErrorCategory::InvalidArgument,
                "content_length_mismatch",
                "request body length does not match Content-Length",
            ),
            Self::Response { request_id } => remote_protocol_error(
                "content_length_mismatch",
                "response body length does not match Content-Length",
                request_id,
            ),
        }
    }
}

fn client_response_budget_exhausted() -> Error {
    Error::framework(
        ErrorCategory::ResourceExhausted,
        "response_byte_budget_exhausted",
        "client response byte budget is exhausted",
    )
}

fn client_response_too_large() -> Error {
    Error::framework(
        ErrorCategory::PayloadTooLarge,
        "response_too_large",
        "response body exceeds the configured limit",
    )
}

async fn read_body_with_role(
    mut body: Incoming,
    content_length: Option<usize>,
    max_body: usize,
    budget: &std::sync::Arc<ByteBudget>,
    role: BodyReadRole<'_>,
) -> Result<(Bytes, BytePermit), Error> {
    if content_length.is_some_and(|length| length > max_body) {
        return Err(role.too_large());
    }
    let initial_reservation = content_length.unwrap_or(0);
    let permit = budget
        .try_reserve(initial_reservation)
        .ok_or_else(|| role.budget_exhausted())?;
    let mut reserved = initial_reservation;
    let mut bytes = Vec::with_capacity(initial_reservation.min(16 * 1024));
    while let Some(frame) =
        std::future::poll_fn(|context| Pin::new(&mut body).poll_frame(context)).await
    {
        let frame = frame.map_err(|error| role.stream_failed(error))?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        let next = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| role.too_large())?;
        if next > max_body {
            return Err(role.too_large());
        }
        if next > reserved {
            let needed = next - reserved;
            let growth = needed.div_ceil(CHUNK_RESERVATION) * CHUNK_RESERVATION;
            if !permit.grow(growth) {
                return Err(role.budget_exhausted());
            }
            reserved += growth;
        }
        bytes.extend_from_slice(&chunk);
    }
    if content_length.is_some_and(|length| length != bytes.len()) {
        return Err(role.content_length_mismatch());
    }
    Ok((Bytes::from(bytes), permit))
}

fn payload_too_large() -> Error {
    Error::framework(
        ErrorCategory::PayloadTooLarge,
        "payload_too_large",
        "request or response body exceeds the configured limit",
    )
}

fn request_budget_exhausted() -> Error {
    Error::framework(
        ErrorCategory::ResourceExhausted,
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

impl HttpBody for GuardedBody {
    type Data = GuardedChunk;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _context: &mut TaskContext<'_>,
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
    use crate::{ErrorCode, ErrorKind, RemoteErrorParts};
    use fusen_contract::{HttpOperation, HttpParameter, MethodId, ServiceSelector};
    use http_body_util::BodyExt;
    use std::sync::LazyLock;

    fn json_method() -> &'static MethodDescriptor {
        static METHOD: LazyLock<MethodDescriptor> = LazyLock::new(|| {
            MethodDescriptor::new(
                MethodId::new(0),
                "get_item",
                HttpOperation::new(
                    Method::GET,
                    "/items",
                    Vec::new(),
                    JSON_CONTENT_TYPE,
                    JSON_CONTENT_TYPE,
                )
                .unwrap(),
            )
            .unwrap()
        });
        &METHOD
    }

    fn json_service() -> &'static ServiceDescriptor {
        static SERVICE: LazyLock<ServiceDescriptor> = LazyLock::new(|| {
            ServiceDescriptor::new(
                ServiceSelector::new("codec-test", None, None).unwrap(),
                vec![json_method().clone()],
            )
            .unwrap()
        });
        &SERVICE
    }

    fn json_contract_with_parameters(
        parameters: Vec<HttpParameter>,
    ) -> (&'static ServiceDescriptor, &'static MethodDescriptor) {
        let method = MethodDescriptor::new(
            MethodId::new(0),
            "get_item",
            HttpOperation::new(
                Method::GET,
                "/items",
                parameters,
                JSON_CONTENT_TYPE,
                JSON_CONTENT_TYPE,
            )
            .unwrap(),
        )
        .unwrap();
        let service = Box::leak(Box::new(
            ServiceDescriptor::new(
                ServiceSelector::new("codec-parameter-test", None, None).unwrap(),
                vec![method],
            )
            .unwrap(),
        ));
        let method = service.method(MethodId::new(0)).unwrap();
        (service, method)
    }

    struct ReturningErrorEncoder(Error);

    impl RequestEncoder for ReturningErrorEncoder {
        fn encode(&self, _request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
            Err(self.0.clone())
        }
    }

    struct FixedBodyEncoder(Bytes);

    impl RequestEncoder for FixedBodyEncoder {
        fn encode(&self, _request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
            Ok(EncodedRequest::new(
                Method::GET,
                "/items",
                HeaderMap::new(),
                self.0.clone(),
            ))
        }
    }

    #[test]
    fn request_ids_are_strict_ascii_tokens() {
        for invalid in ["", "has space", "中文", &"x".repeat(65)] {
            assert!(validate_request_id(invalid).is_err());
        }
        assert!(validate_request_id("request_1.a-b").is_ok());
    }

    #[test]
    fn only_one_valid_request_id_is_trusted_for_early_error_responses() {
        let mut headers = HeaderMap::new();
        headers.insert(REQUEST_ID, HeaderValue::from_static("request_1.a-b"));
        assert_eq!(validated_request_id_header(&headers), Some("request_1.a-b"));

        headers.append(REQUEST_ID, HeaderValue::from_static("request-2"));
        assert_eq!(validated_request_id_header(&headers), None);

        let mut invalid = HeaderMap::new();
        invalid.insert(REQUEST_ID, HeaderValue::from_static("invalid request"));
        assert_eq!(validated_request_id_header(&invalid), None);
    }

    #[test]
    fn endpoint_uris_preserve_http_and_https_schemes() {
        for (endpoint, expected) in [
            (
                "http://example.com:8080/base",
                "http://example.com:8080/base/items?page=2",
            ),
            (
                "https://example.com/base",
                "https://example.com/base/items?page=2",
            ),
        ] {
            let endpoint: ServiceEndpoint = endpoint.parse().unwrap();
            let uri = endpoint_uri(&endpoint, "/items?page=2").unwrap();
            assert_eq!(uri.to_string(), expected);
        }
    }

    #[test]
    fn request_encoder_cannot_change_the_canonical_http_method() {
        let error =
            validate_encoded_request(json_method(), &Method::POST, "/items", &HeaderMap::new())
                .unwrap_err();
        assert_eq!(error.category(), ErrorCategory::InvalidArgument);
        assert_eq!(error.code().as_str(), "invalid_encoded_request_method");
        validate_encoded_request(json_method(), &Method::GET, "/items", &HeaderMap::new()).unwrap();
    }

    #[test]
    fn request_encoder_errors_are_always_attributed_locally() {
        let remote = Error::from_remote_parts(RemoteErrorParts {
            kind: ErrorKind::Framework,
            category: ErrorCategory::Unavailable,
            code: ErrorCode::framework("stored_remote_error"),
            message: "stored remote failure".to_owned(),
            status: StatusCode::SERVICE_UNAVAILABLE,
            request_id: "previous-request".to_owned(),
            details: None,
            retry_hint: RetryHint::Never,
        });
        assert_eq!(remote.origin(), ErrorOrigin::Remote);

        let error = match encode_request_template(
            &ReturningErrorEncoder(remote),
            json_service(),
            json_method(),
            &Arguments::new(),
            &HeaderMap::new(),
            16,
            &ByteBudget::new(16),
        ) {
            Ok(_) => panic!("request encoder errors must remain local to this runtime"),
            Err(error) => error,
        };

        assert_eq!(error.origin(), ErrorOrigin::Local);
        assert_eq!(error.code().as_str(), "stored_remote_error");
    }

    #[test]
    fn buffered_response_debug_contains_only_http_semantics() {
        let response = BufferedResponse::new(
            StatusCode::OK,
            Version::HTTP_2,
            HeaderMap::new(),
            Bytes::from_static(b"body"),
            "private-request-id".to_owned(),
            true,
        );

        let debug = format!("{response:?}");
        assert!(debug.contains("status"));
        assert!(debug.contains("version"));
        assert!(debug.contains("header_count"));
        assert!(debug.contains("body_length"));
        assert!(!debug.contains("private-request-id"));
        assert!(!debug.contains("invocation_controls"));
    }

    #[test]
    fn http_query_encoding_obeys_declared_cardinality() {
        let mut query = Vec::new();
        append_query(
            &mut query,
            "enabled",
            HttpParameterCardinality::Scalar,
            &Value::Null,
        )
        .unwrap();
        append_query(
            &mut query,
            "enabled",
            HttpParameterCardinality::Scalar,
            &Value::Bool(true),
        )
        .unwrap();
        append_query(
            &mut query,
            "tag",
            HttpParameterCardinality::Repeated,
            &serde_json::json!([]),
        )
        .unwrap();
        append_query(
            &mut query,
            "tag",
            HttpParameterCardinality::Repeated,
            &serde_json::json!(["one", "two words"]),
        )
        .unwrap();
        assert_eq!(query, ["enabled=true", "tag=one", "tag=two%20words"]);

        let scalar_array = append_query(
            &mut Vec::new(),
            "enabled",
            HttpParameterCardinality::Scalar,
            &serde_json::json!([true]),
        )
        .unwrap_err();
        assert_eq!(scalar_array.code().as_str(), "invalid_http_parameter");
        assert!(scalar_array.message().contains("enabled"));
        assert!(scalar_array.message().contains("#[param(query, repeated)]"));

        let repeated_scalar = append_query(
            &mut Vec::new(),
            "tag",
            HttpParameterCardinality::Repeated,
            &Value::String("one".into()),
        )
        .unwrap_err();
        assert_eq!(repeated_scalar.code().as_str(), "invalid_http_parameter");
        assert!(repeated_scalar.message().contains("tag"));
        assert!(repeated_scalar.message().contains("remove `repeated`"));
    }

    #[test]
    fn parameter_maps_expand_repeated_values_ignore_null_and_reject_conflicts() {
        let mut query = Vec::new();
        append_query_map(
            &mut query,
            &serde_json::json!({
                "ignored": null,
                "page": 2,
                "tag": ["one", "two words"]
            }),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(query, ["page=2", "tag=one", "tag=two%20words"]);

        let query_conflict = append_query_map(
            &mut vec!["page=1".to_owned()],
            &serde_json::json!({"page": 2}),
            &HashSet::new(),
        )
        .unwrap_err();
        assert_eq!(query_conflict.code().as_str(), "duplicate_query_parameter");

        let mut headers = HeaderMap::new();
        append_header_map(
            &mut headers,
            &serde_json::json!({
                "x-ignored": null,
                "x-scope": "user",
                "x-tag": ["one", "two"]
            }),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(headers.get("x-scope").unwrap(), "user");
        assert_eq!(
            headers
                .get_all("x-tag")
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert!(!headers.contains_key("x-ignored"));

        let header_conflict = append_header_map(
            &mut HeaderMap::from_iter([(
                HeaderName::from_static("x-scope"),
                HeaderValue::from_static("call"),
            )]),
            &serde_json::json!({"x-scope": "map"}),
            &HashSet::new(),
        )
        .unwrap_err();
        assert_eq!(header_conflict.code().as_str(), "header_binding_conflict");

        let control_conflict = append_header_map(
            &mut HeaderMap::new(),
            &serde_json::json!({"x-fusen-attempt": "2"}),
            &HashSet::new(),
        )
        .unwrap_err();
        assert_eq!(control_conflict.code().as_str(), "header_binding_conflict");

        let explicit_query = HashSet::from(["page"]);
        let declared_query_conflict = append_query_map(
            &mut Vec::new(),
            &serde_json::json!({"page": 2}),
            &explicit_query,
        )
        .unwrap_err();
        assert_eq!(
            declared_query_conflict.code().as_str(),
            "duplicate_query_parameter"
        );
        append_query_map(
            &mut vec!["page=1".to_owned()],
            &serde_json::json!({"page": null, "tags": []}),
            &explicit_query,
        )
        .unwrap();

        let explicit_headers = HashSet::from([HeaderName::from_static("x-scope")]);
        let declared_header_conflict = append_header_map(
            &mut HeaderMap::new(),
            &serde_json::json!({"x-scope": "map"}),
            &explicit_headers,
        )
        .unwrap_err();
        assert_eq!(
            declared_header_conflict.code().as_str(),
            "header_binding_conflict"
        );
        append_header_map(
            &mut HeaderMap::from_iter([(
                HeaderName::from_static("x-scope"),
                HeaderValue::from_static("call"),
            )]),
            &serde_json::json!({"x-scope": null, "x-tags": []}),
            &explicit_headers,
        )
        .unwrap();
    }

    #[test]
    fn json_request_rejects_declared_parameter_and_call_header_conflicts_independent_of_values() {
        let (service, method) = json_contract_with_parameters(vec![
            HttpParameter::new(
                "query_map",
                HttpParameterSource::QueryMap,
                HttpParameterCardinality::Scalar,
            )
            .unwrap(),
            HttpParameter::new(
                "page",
                HttpParameterSource::Query,
                HttpParameterCardinality::Scalar,
            )
            .unwrap(),
            HttpParameter::new(
                "header_map",
                HttpParameterSource::HeaderMap,
                HttpParameterCardinality::Scalar,
            )
            .unwrap(),
            HttpParameter::new(
                "x-scope",
                HttpParameterSource::Header,
                HttpParameterCardinality::Scalar,
            )
            .unwrap(),
            HttpParameter::new(
                "session",
                HttpParameterSource::Cookie,
                HttpParameterCardinality::Scalar,
            )
            .unwrap(),
        ]);

        let mut arguments = Arguments::new();
        arguments.insert("query_map".into(), serde_json::json!({"page": 2}));
        arguments.insert("page".into(), Value::Null);
        let error = encode_json_request(RequestEncoding::new(
            service,
            method,
            &arguments,
            &HeaderMap::new(),
        ))
        .unwrap_err();
        assert_eq!(error.code().as_str(), "duplicate_query_parameter");

        let mut arguments = Arguments::new();
        arguments.insert("query_map".into(), Value::Null);
        arguments.insert("page".into(), serde_json::json!(1));
        arguments.insert("header_map".into(), serde_json::json!({"x-scope": "map"}));
        arguments.insert("x-scope".into(), Value::Null);
        let error = encode_json_request(RequestEncoding::new(
            service,
            method,
            &arguments,
            &HeaderMap::new(),
        ))
        .unwrap_err();
        assert_eq!(error.code().as_str(), "header_binding_conflict");

        arguments.insert("header_map".into(), Value::Null);
        let encoded = encode_json_request(RequestEncoding::new(
            service,
            method,
            &arguments,
            &HeaderMap::new(),
        ))
        .unwrap();
        assert_eq!(encoded.path_and_query(), "/items?page=1");
        assert!(!encoded.headers().contains_key("x-scope"));
        assert!(!encoded.headers().contains_key(COOKIE));

        let call_headers = HeaderMap::from_iter([(
            HeaderName::from_static("x-scope"),
            HeaderValue::from_static("call"),
        )]);
        let error = encode_json_request(RequestEncoding::new(
            service,
            method,
            &arguments,
            &call_headers,
        ))
        .unwrap_err();
        assert_eq!(error.code().as_str(), "header_binding_conflict");

        let call_headers =
            HeaderMap::from_iter([(COOKIE, HeaderValue::from_static("session=call"))]);
        let error = encode_json_request(RequestEncoding::new(
            service,
            method,
            &arguments,
            &call_headers,
        ))
        .unwrap_err();
        assert_eq!(error.code().as_str(), "header_binding_conflict");
    }

    #[test]
    fn cookie_values_reject_cookie_pair_injection() {
        assert_eq!(
            cookie_text(&Value::String("safe=value".into()), "session").unwrap(),
            "safe=value"
        );
        for value in ["x; admin=true", "x,admin", "quoted\"", "has space"] {
            let error = cookie_text(&Value::String(value.into()), "session").unwrap_err();
            assert_eq!(error.category(), ErrorCategory::InvalidArgument);
            assert_eq!(error.code().as_str(), "invalid_cookie_parameter");
        }
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
    fn response_content_type_must_be_one_json_media_type() {
        let mut invalid_utf8 = HeaderMap::new();
        invalid_utf8.insert(CONTENT_TYPE, HeaderValue::from_bytes(b"\x80").unwrap());
        let mut duplicate = HeaderMap::new();
        duplicate.append(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
        duplicate.append(CONTENT_TYPE, HeaderValue::from_static("text/plain"));

        for headers in [HeaderMap::new(), invalid_utf8, duplicate] {
            let error =
                validate_json_response_content_type(&headers, JSON_CONTENT_TYPE, "request-1")
                    .unwrap_err();
            assert_eq!(error.origin(), ErrorOrigin::Remote);
            assert_eq!(error.category(), ErrorCategory::DataLoss);
            assert_eq!(error.code().as_str(), "invalid_content_type");
            assert_eq!(error.request_id(), Some("request-1"));
        }

        for content_type in [
            HeaderValue::from_static(JSON_CONTENT_TYPE),
            HeaderValue::from_static("application/vnd.example+json; charset=utf-8"),
        ] {
            let headers = HeaderMap::from_iter([(CONTENT_TYPE, content_type)]);
            validate_json_response_content_type(&headers, JSON_CONTENT_TYPE, "request-1").unwrap();
        }
    }

    #[test]
    fn json_binding_accepts_only_application_json_family_media_types() {
        for media_type in [
            "application/json",
            "application/json; charset=utf-8",
            "application/problem+json",
            "application/vnd.example+json; profile=compact",
        ] {
            assert!(
                is_json_media_type(media_type),
                "expected {media_type} to be JSON-compatible"
            );
        }

        for media_type in [
            "text/json",
            "text/plain",
            "application/*",
            "application/*+json",
            "application/json+zip",
        ] {
            assert!(
                !is_json_media_type(media_type),
                "expected {media_type} to be rejected by http-json-v1"
            );
        }

        let headers = HeaderMap::from_iter([(CONTENT_TYPE, HeaderValue::from_static("text/json"))]);
        let error = validate_content_type(&headers, JSON_CONTENT_TYPE, true).unwrap_err();
        assert_eq!(error.category(), ErrorCategory::InvalidArgument);
        assert_eq!(error.code().as_str(), "invalid_content_type");
    }

    #[test]
    fn decoder_headers_keep_http_semantics_but_hide_runtime_controls() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("4"));
        headers.insert(REQUEST_ID, HeaderValue::from_static("request-1"));
        headers.insert(ATTEMPT, HeaderValue::from_static("1"));
        headers.insert(CONNECTION, HeaderValue::from_static("close"));
        headers.insert("x-application", HeaderValue::from_static("visible"));

        let visible = response_headers_for_decoder(headers);
        assert_eq!(visible.get(CONTENT_TYPE).unwrap(), JSON_CONTENT_TYPE);
        assert_eq!(visible.get(CONTENT_LENGTH).unwrap(), "4");
        assert_eq!(visible.get("x-application").unwrap(), "visible");
        assert!(!visible.contains_key(REQUEST_ID));
        assert!(!visible.contains_key(ATTEMPT));
        assert!(!visible.contains_key(CONNECTION));
    }

    #[test]
    fn json_codec_accepts_204_and_205_without_content_type() {
        for status in [StatusCode::NO_CONTENT, StatusCode::RESET_CONTENT] {
            let response = ResponseDecoder::decode(
                &JsonCodec,
                json_method(),
                BufferedResponse::new(
                    status,
                    Version::HTTP_11,
                    HeaderMap::new(),
                    Bytes::from_static(b"null"),
                    "request-1".to_owned(),
                    false,
                ),
            )
            .unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(response.result_bytes(), &Bytes::from_static(b"null"));
        }
    }

    #[test]
    fn invalid_response_content_length_is_a_terminal_remote_protocol_error() {
        let mut headers = HeaderMap::new();
        headers.append(CONTENT_LENGTH, HeaderValue::from_static("12"));
        headers.append(CONTENT_LENGTH, HeaderValue::from_static("13"));

        let error = parse_response_content_length(&headers, "request-1").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.origin(), ErrorOrigin::Remote);
        assert_eq!(error.category(), ErrorCategory::DataLoss);
        assert_eq!(error.request_id(), Some("request-1"));
        assert_eq!(error.retry_hint(), RetryHint::Never);
    }

    #[test]
    fn unsupported_response_http_versions_are_remote_protocol_errors() {
        for version in [Version::HTTP_09, Version::HTTP_10, Version::HTTP_3] {
            let error = validate_response_http_version(
                version,
                "request-1",
                &HeaderMap::from_iter([
                    (CONTENT_TYPE, HeaderValue::from_static("text/plain")),
                    (
                        HeaderName::from_static("x-application"),
                        HeaderValue::from_static("visible"),
                    ),
                ]),
            )
            .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Framework);
            assert_eq!(error.origin(), ErrorOrigin::Remote);
            assert_eq!(error.category(), ErrorCategory::DataLoss);
            assert_eq!(error.code().as_str(), "invalid_http_version");
            assert_eq!(error.request_id(), Some("request-1"));
            assert_eq!(error.headers().get("x-application").unwrap(), "visible");
            assert!(!error.headers().contains_key(CONTENT_TYPE));
        }

        for version in [Version::HTTP_11, Version::HTTP_2] {
            validate_response_http_version(version, "request-1", &HeaderMap::new()).unwrap();
        }
    }

    #[test]
    fn response_body_failures_keep_remote_and_local_ownership_separate() {
        let role = BodyReadRole::Response {
            request_id: "request-1",
        };

        let stream = role.stream_failed(std::io::Error::other("controlled failure"));
        assert_eq!(stream.kind(), ErrorKind::Framework);
        assert_eq!(stream.origin(), ErrorOrigin::Remote);
        assert_eq!(stream.category(), ErrorCategory::DataLoss);
        assert_eq!(stream.retry_hint(), RetryHint::Never);

        let mismatch = role.content_length_mismatch();
        assert_eq!(mismatch.origin(), ErrorOrigin::Remote);
        assert_eq!(mismatch.category(), ErrorCategory::DataLoss);

        let too_large = role.too_large();
        assert_eq!(too_large.origin(), ErrorOrigin::Local);
        assert_eq!(too_large.category(), ErrorCategory::PayloadTooLarge);

        let exhausted = role.budget_exhausted();
        assert_eq!(exhausted.origin(), ErrorOrigin::Local);
        assert_eq!(exhausted.category(), ErrorCategory::ResourceExhausted);
    }

    #[test]
    fn decoded_response_body_is_revalidated_and_fully_budgeted() {
        let budget = ByteBudget::new(8);
        let permit = budget.try_reserve(2).unwrap();
        let mut response = Response::from_json_bytes(Bytes::from_static(b"12345678"));
        hold_decoded_response_budget(&mut response, permit, 8).unwrap();
        assert_eq!(budget.used(), 8);
        drop(response);
        assert_eq!(budget.used(), 0);

        let permit = budget.try_reserve(2).unwrap();
        let mut oversized = Response::from_json_bytes(Bytes::from_static(b"12345"));
        let error = hold_decoded_response_budget(&mut oversized, permit, 4).unwrap_err();
        assert_eq!(error.category(), ErrorCategory::PayloadTooLarge);
        assert_eq!(error.code().as_str(), "response_too_large");
        assert_eq!(budget.used(), 0);

        let constrained = ByteBudget::new(4);
        let permit = constrained.try_reserve(2).unwrap();
        let mut expanded = Response::from_json_bytes(Bytes::from_static(b"12345"));
        let error = hold_decoded_response_budget(&mut expanded, permit, 8).unwrap_err();
        assert_eq!(error.category(), ErrorCategory::ResourceExhausted);
        assert_eq!(error.code().as_str(), "response_byte_budget_exhausted");
        assert_eq!(constrained.used(), 0);
    }

    #[test]
    fn retries_for_non_replayable_methods_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(ATTEMPT, HeaderValue::from_static("2"));
        assert_eq!(
            validate_attempt(
                parse_request_control(&headers, Duration::from_secs(1))
                    .unwrap()
                    .attempt,
                false,
            )
            .unwrap_err()
            .code()
            .as_str(),
            "invalid_attempt"
        );
    }

    #[test]
    fn encoded_request_body_stops_at_the_configured_limit() {
        let budget = ByteBudget::new(1024);
        let error = encode_request_template(
            &FixedBodyEncoder(Bytes::from(vec![b'x'; 128])),
            json_service(),
            json_method(),
            &Arguments::new(),
            &HeaderMap::new(),
            16,
            &budget,
        )
        .unwrap_err();
        assert_eq!(error.code().as_str(), "request_too_large");
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn encoded_request_releases_budget_when_reservation_is_exhausted() {
        let budget = ByteBudget::new(8);
        let error = encode_request_template(
            &FixedBodyEncoder(Bytes::from(vec![b'x'; 128])),
            json_service(),
            json_method(),
            &Arguments::new(),
            &HeaderMap::new(),
            1024,
            &budget,
        )
        .unwrap_err();
        assert_eq!(error.code().as_str(), "request_byte_budget_exhausted");
        assert_eq!(budget.used(), 0);
    }

    #[tokio::test]
    async fn response_budget_covers_serialization_and_body() {
        let budget = ByteBudget::new(4);
        let response = Response::success_with_budget("ok", 4, 0, &budget).unwrap();
        assert_eq!(budget.used(), 4);
        let response = encode_success(response, JSON_CONTENT_TYPE, false, 4, &budget).unwrap();
        assert_eq!(response.body().size_hint().exact(), Some(4));
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(body, r#""ok""#);
        assert_eq!(budget.used(), 0);
    }

    #[tokio::test]
    async fn head_204_and_205_successes_suppress_the_body_and_content_type() {
        for (status, head) in [
            (StatusCode::OK, true),
            (StatusCode::NO_CONTENT, false),
            (StatusCode::RESET_CONTENT, false),
        ] {
            let budget = ByteBudget::new(16);
            let mut response = Response::success_with_budget("ok", 16, 0, &budget).unwrap();
            response.set_status(status).unwrap();
            let encoded = encode_success(response, JSON_CONTENT_TYPE, head, 16, &budget).unwrap();
            assert_eq!(encoded.status(), status);
            assert!(encoded.headers().get(CONTENT_TYPE).is_none());
            assert_eq!(encoded.body().size_hint().exact(), Some(0));
            assert!(
                encoded
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .is_empty()
            );
            assert_eq!(budget.used(), 0);
        }
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
        let mut response = Response::success_with_budget("ok", 1024, 0, &budget).unwrap();
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

        let encoded = encode_success(response, JSON_CONTENT_TYPE, false, 1024, &budget).unwrap();
        assert!(encoded.headers().get(CONTENT_LENGTH).is_none());
        assert!(encoded.headers().get(TRANSFER_ENCODING).is_none());
        assert!(encoded.headers().get(REQUEST_ID).is_none());
        assert_eq!(
            encoded.headers().get(CONTENT_TYPE).unwrap(),
            JSON_CONTENT_TYPE
        );
    }
}
