use super::{
    EMERGENCY_PROBLEM_LIMIT, GuardedBody, LimitedWriter, PROBLEM_CONTENT_TYPE, REQUEST_ID,
    request_id_is_valid, response_headers_without_control,
};
use crate::{
    Error, ErrorCategory, ErrorCode, ErrorDetails, ErrorKind, ErrorOrigin, RemoteErrorParts,
    RetryHint,
};
use bytes::Bytes;
use http::{
    HeaderMap, HeaderValue, StatusCode,
    header::{CONTENT_TYPE, RETRY_AFTER},
};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProblemDetails {
    #[serde(rename = "type")]
    type_uri: String,
    title: String,
    status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<ErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(default)]
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<ErrorDetails>,
}

impl ProblemDetails {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        type_uri: impl Into<String>,
        title: impl Into<String>,
        status: u16,
        detail: Option<String>,
        instance: Option<String>,
        code: ErrorCode,
        request_id: impl Into<String>,
        retryable: bool,
        details: Option<ErrorDetails>,
    ) -> Self {
        Self {
            type_uri: type_uri.into(),
            title: title.into(),
            status,
            detail,
            instance,
            code: Some(code),
            request_id: Some(request_id.into()),
            retryable,
            details,
        }
    }

    fn without_optional_fields(&self) -> Self {
        Self {
            type_uri: self.type_uri.clone(),
            title: self.title.clone(),
            status: self.status,
            detail: None,
            instance: None,
            code: self.code.clone(),
            request_id: self.request_id.clone(),
            retryable: self.retryable,
            details: None,
        }
    }

    #[allow(dead_code)] // Used by the fuzz-support include mirror.
    pub(crate) const fn status(&self) -> u16 {
        self.status
    }

    #[allow(dead_code)] // Used by the fuzz-support include mirror.
    pub(crate) fn request_id(&self) -> &str {
        self.request_id.as_deref().unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProblemSemantics {
    kind: ErrorKind,
    category: ErrorCategory,
}

pub(crate) fn encode_problem(
    error: &Error,
    request_id: &str,
    instance: Option<String>,
    invocation_controls: bool,
) -> http::Response<GuardedBody> {
    let (mut problem, status) = problem_from_error(error, request_id, instance);
    if !invocation_controls {
        problem.request_id = None;
    }
    let body = bounded_problem(&problem);
    let mut response = http::Response::new(GuardedBody::new(body, None));
    *response.status_mut() = status;
    if error.origin() == ErrorOrigin::Local {
        *response.headers_mut() = response_headers_without_control(error.headers().clone());
    }
    if error.kind() == ErrorKind::Framework
        && error.origin() == ErrorOrigin::Local
        && let Some(delay) = error.retry_hint().retry_after()
    {
        let seconds = delay
            .as_secs()
            .saturating_add(u64::from(delay.subsec_nanos() != 0));
        response.headers_mut().insert(
            RETRY_AFTER,
            HeaderValue::from_str(&seconds.to_string())
                .expect("retry delay seconds are valid header text"),
        );
    }
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(PROBLEM_CONTENT_TYPE));
    if invocation_controls {
        response.headers_mut().insert(
            REQUEST_ID,
            HeaderValue::from_str(request_id).expect("validated request ID is valid header text"),
        );
    }
    response
}

pub(super) fn decode_problem(
    status: StatusCode,
    expected_request_id: &str,
    body: &[u8],
    headers: HeaderMap,
    strict_controls: bool,
) -> Error {
    let problem = match serde_json::from_slice::<ProblemDetails>(body) {
        Ok(problem) => problem,
        Err(_) => return status_error(status, expected_request_id, headers),
    };
    let semantics = match validate_problem(status, expected_request_id, &problem, strict_controls) {
        Ok(semantics) => semantics,
        Err(error) => return error.with_headers(headers),
    };
    let retry_hint = normalized_retry_hint(semantics.kind, status, &headers, SystemTime::now());
    let details = (semantics.kind == ErrorKind::Application)
        .then_some(problem.details)
        .flatten();
    Error::from_remote_parts(RemoteErrorParts {
        kind: semantics.kind,
        category: semantics.category,
        code: problem
            .code
            .unwrap_or_else(|| ErrorCode::framework("remote_http_error")),
        message: problem
            .detail
            .unwrap_or_else(|| "remote service returned an error".to_owned()),
        status,
        request_id: problem
            .request_id
            .unwrap_or_else(|| expected_request_id.to_owned()),
        details,
        retry_hint,
    })
    .with_headers(headers)
}

pub(super) fn decode_head_error(
    status: StatusCode,
    expected_request_id: &str,
    headers: HeaderMap,
) -> Error {
    let retry_hint =
        normalized_retry_hint(ErrorKind::Framework, status, &headers, SystemTime::now());
    Error::from_remote_parts(RemoteErrorParts {
        kind: ErrorKind::Framework,
        category: category_from_status(status),
        code: ErrorCode::framework("remote_head_error"),
        message: status
            .canonical_reason()
            .unwrap_or("remote HEAD request failed")
            .to_owned(),
        status,
        request_id: expected_request_id.to_owned(),
        details: None,
        retry_hint,
    })
    .with_headers(headers)
}

pub(super) fn validate_response_request_id(
    required: bool,
    headers: &HeaderMap,
    expected_request_id: &str,
) -> Result<(), Error> {
    if !required {
        return Ok(());
    }
    let mut values = headers.get_all(&REQUEST_ID).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(invalid_response_request_id(
            "response x-request-id must appear at most once",
            expected_request_id,
            headers,
        ));
    }
    let Some(value) = value else {
        return if required {
            Err(invalid_response_request_id(
                "response is missing negotiated x-request-id",
                expected_request_id,
                headers,
            ))
        } else {
            Ok(())
        };
    };
    let value = value
        .to_str()
        .ok()
        .filter(|value| request_id_is_valid(value));
    if value == Some(expected_request_id) {
        Ok(())
    } else {
        Err(invalid_response_request_id(
            "response x-request-id is invalid or does not match the logical invocation",
            expected_request_id,
            headers,
        ))
    }
}

pub(crate) fn remote_protocol_error(
    code: &'static str,
    message: &'static str,
    request_id: &str,
) -> Error {
    Error::from_remote_parts(RemoteErrorParts {
        kind: ErrorKind::Framework,
        category: ErrorCategory::DataLoss,
        code: ErrorCode::framework(code),
        message: message.to_owned(),
        status: StatusCode::BAD_GATEWAY,
        request_id: request_id.to_owned(),
        details: None,
        retry_hint: RetryHint::Never,
    })
}

fn invalid_response_request_id(
    message: &'static str,
    expected_request_id: &str,
    headers: &HeaderMap,
) -> Error {
    remote_protocol_error("invalid_response_request_id", message, expected_request_id)
        .with_headers(response_headers_without_control(headers.clone()))
}

pub(crate) fn problem_from_error(
    error: &Error,
    request_id: &str,
    instance: Option<String>,
) -> (ProblemDetails, StatusCode) {
    let (kind, category_name, status) = if error.kind() == ErrorKind::Application {
        (ErrorKind::Application, "application", error.status())
    } else if let Some(category_name) = framework_category_name(error.category())
        && let Some(status) = error.category().canonical_status()
    {
        (ErrorKind::Framework, category_name, status)
    } else {
        (
            ErrorKind::Framework,
            "internal",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
    };
    let detail = if kind == ErrorKind::Framework && category_name == "internal" {
        Some("Internal server error".to_owned())
    } else {
        Some(error.message().to_owned())
    };
    let details = (kind == ErrorKind::Application && error.origin() == ErrorOrigin::Local)
        .then(|| error.details().cloned())
        .flatten();
    (
        ProblemDetails {
            type_uri: format!("urn:fusen:error:{category_name}:{}", error.code()),
            title: status
                .canonical_reason()
                .unwrap_or("Service Invocation Error")
                .to_owned(),
            status: status.as_u16(),
            detail,
            instance,
            code: Some(error.code().clone()),
            request_id: Some(request_id.to_owned()),
            retryable: kind == ErrorKind::Framework && error.retry_hint().is_retryable(),
            details,
        },
        status,
    )
}

pub(crate) fn bounded_problem(problem: &ProblemDetails) -> Bytes {
    let mut writer = LimitedWriter::new(EMERGENCY_PROBLEM_LIMIT);
    match serde_json::to_writer(&mut writer, problem) {
        Ok(()) => Bytes::from(writer.into_inner()),
        _ => {
            let minimal = problem.without_optional_fields();
            let mut writer = LimitedWriter::new(EMERGENCY_PROBLEM_LIMIT);
            serde_json::to_writer(&mut writer, &minimal)
                .expect("validated problem metadata fits the emergency response budget");
            Bytes::from(writer.into_inner())
        }
    }
}

fn validate_problem(
    status: StatusCode,
    expected_request_id: &str,
    problem: &ProblemDetails,
    strict_controls: bool,
) -> Result<ProblemSemantics, Error> {
    let reserved = problem.type_uri.starts_with("urn:fusen:error:");
    if !status.is_client_error() && !status.is_server_error() {
        return Err(remote_protocol_error(
            "invalid_problem_status",
            "Problem Details requires an HTTP 4xx or 5xx status",
            expected_request_id,
        ));
    }
    if problem.status != status.as_u16() {
        return if reserved {
            Err(remote_protocol_error(
                "invalid_problem_status",
                "reserved Problem Details status must match the HTTP status",
                expected_request_id,
            ))
        } else {
            Err(status_error(status, expected_request_id, HeaderMap::new()))
        };
    }
    if strict_controls
        && problem.request_id.as_deref().is_none_or(|request_id| {
            !request_id_is_valid(request_id) || request_id != expected_request_id
        })
    {
        return Err(remote_protocol_error(
            "invalid_problem_request_id",
            "Problem Details request_id is invalid or does not match the logical invocation",
            expected_request_id,
        ));
    }
    let semantics = parse_problem_type(problem, expected_request_id)?;
    if semantics.kind == ErrorKind::Framework
        && problem.type_uri.starts_with("urn:fusen:error:")
        && semantics.category.canonical_status() != Some(status)
    {
        return Err(remote_protocol_error(
            "invalid_problem_status",
            "Problem Details category does not match its canonical HTTP status",
            expected_request_id,
        ));
    }
    Ok(semantics)
}

fn parse_problem_type(
    problem: &ProblemDetails,
    expected_request_id: &str,
) -> Result<ProblemSemantics, Error> {
    const PREFIX: &str = "urn:fusen:error:";
    if problem
        .type_uri
        .get(..PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX) && prefix != PREFIX)
    {
        return Err(invalid_problem_type(expected_request_id));
    }
    if let Some(value) = problem.type_uri.strip_prefix(PREFIX) {
        let Some((category, code)) = value.split_once(':') else {
            return Err(invalid_problem_type(expected_request_id));
        };
        if category.is_empty()
            || code.is_empty()
            || code.contains(':')
            || problem
                .code
                .as_ref()
                .is_none_or(|problem_code| code != problem_code.as_str())
        {
            return Err(invalid_problem_type(expected_request_id));
        }
        if category == "application" {
            return Ok(ProblemSemantics {
                kind: ErrorKind::Application,
                category: category_from_status(
                    StatusCode::from_u16(problem.status)
                        .expect("validated Problem status is an HTTP status"),
                ),
            });
        }
        let Some(category) = framework_category_from_name(category) else {
            return Err(invalid_problem_type(expected_request_id));
        };
        return Ok(ProblemSemantics {
            kind: ErrorKind::Framework,
            category,
        });
    }
    if !is_valid_uri_reference(&problem.type_uri) {
        return Err(invalid_problem_type(expected_request_id));
    }
    Ok(ProblemSemantics {
        kind: if problem.status < 500 {
            ErrorKind::Application
        } else {
            ErrorKind::Framework
        },
        category: category_from_status(
            StatusCode::from_u16(problem.status)
                .expect("validated Problem status is an HTTP status"),
        ),
    })
}

fn is_valid_uri_reference(value: &str) -> bool {
    let (without_fragment, fragment) = match value.split_once('#') {
        Some((head, fragment)) if !fragment.contains('#') => (head, Some(fragment)),
        Some(_) => return false,
        None => (value, None),
    };
    if fragment.is_some_and(|fragment| !valid_uri_component(fragment, is_query_char)) {
        return false;
    }
    let (main, query) = match without_fragment.split_once('?') {
        Some((main, query)) => (main, Some(query)),
        None => (without_fragment, None),
    };
    if query.is_some_and(|query| !valid_uri_component(query, is_query_char)) {
        return false;
    }

    let first_slash = main.find('/').unwrap_or(main.len());
    if let Some(colon) = main.find(':')
        && colon < first_slash
        && is_valid_scheme(&main[..colon])
    {
        return is_valid_hier_part(&main[colon + 1..]);
    }
    is_valid_relative_part(main)
}

fn is_valid_scheme(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

fn is_valid_hier_part(value: &str) -> bool {
    if let Some(authority_and_path) = value.strip_prefix("//") {
        return is_valid_authority_and_path(authority_and_path);
    }
    if value.starts_with('/') {
        return is_valid_absolute_path(value);
    }
    value.is_empty() || is_valid_rootless_path(value, true)
}

fn is_valid_relative_part(value: &str) -> bool {
    if let Some(authority_and_path) = value.strip_prefix("//") {
        return is_valid_authority_and_path(authority_and_path);
    }
    if value.starts_with('/') {
        return is_valid_absolute_path(value);
    }
    value.is_empty() || is_valid_rootless_path(value, false)
}

fn is_valid_authority_and_path(value: &str) -> bool {
    let (authority, path) = value
        .find('/')
        .map_or((value, ""), |index| (&value[..index], &value[index..]));
    is_valid_authority(authority) && is_valid_abempty_path(path)
}

fn is_valid_authority(value: &str) -> bool {
    let host_and_port = match value.rsplit_once('@') {
        Some((userinfo, host_and_port)) => {
            if userinfo.contains('@') || !valid_uri_component(userinfo, is_userinfo_char) {
                return false;
            }
            host_and_port
        }
        None => value,
    };

    if let Some(ip_literal) = host_and_port.strip_prefix('[') {
        let Some(closing) = ip_literal.find(']') else {
            return false;
        };
        let host = &ip_literal[..closing];
        let suffix = &ip_literal[closing + 1..];
        return is_valid_ip_literal(host)
            && (suffix.is_empty()
                || suffix
                    .strip_prefix(':')
                    .is_some_and(|port| port.bytes().all(|byte| byte.is_ascii_digit())));
    }
    if host_and_port.contains(['[', ']']) {
        return false;
    }
    let (host, port) = match host_and_port.rsplit_once(':') {
        Some((host, port)) => {
            if host.contains(':') {
                return false;
            }
            (host, Some(port))
        }
        None => (host_and_port, None),
    };
    valid_uri_component(host, is_reg_name_char)
        && port.is_none_or(|port| port.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_valid_ip_literal(value: &str) -> bool {
    value.parse::<std::net::Ipv6Addr>().is_ok() || is_valid_ipv_future(value)
}

fn is_valid_ipv_future(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('v').or_else(|| value.strip_prefix('V')) else {
        return false;
    };
    let Some((version, address)) = rest.split_once('.') else {
        return false;
    };
    !version.is_empty()
        && version.bytes().all(|byte| byte.is_ascii_hexdigit())
        && !address.is_empty()
        && address
            .bytes()
            .all(|byte| is_unreserved(byte) || is_sub_delim(byte) || byte == b':')
}

fn is_valid_abempty_path(value: &str) -> bool {
    (value.is_empty() || value.starts_with('/'))
        && value
            .strip_prefix('/')
            .unwrap_or_default()
            .split('/')
            .all(|segment| valid_uri_component(segment, is_pchar))
}

fn is_valid_absolute_path(value: &str) -> bool {
    let Some(path) = value.strip_prefix('/') else {
        return false;
    };
    if path.is_empty() {
        return true;
    }
    let mut segments = path.split('/');
    let first = segments.next().expect("a non-empty path has one segment");
    !first.is_empty()
        && valid_uri_component(first, is_pchar)
        && segments.all(|segment| valid_uri_component(segment, is_pchar))
}

fn is_valid_rootless_path(value: &str, first_segment_allows_colon: bool) -> bool {
    let mut segments = value.split('/');
    let first = segments.next().expect("a rootless path has one segment");
    !first.is_empty()
        && valid_uri_component(first, |byte| {
            is_pchar(byte) && (first_segment_allows_colon || byte != b':')
        })
        && segments.all(|segment| valid_uri_component(segment, is_pchar))
}

fn valid_uri_component(value: &str, allowed: impl Fn(u8) -> bool) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if bytes
                .get(index + 1..index + 3)
                .is_none_or(|hex| !hex.iter().all(u8::is_ascii_hexdigit))
            {
                return false;
            }
            index += 3;
        } else if allowed(bytes[index]) {
            index += 1;
        } else {
            return false;
        }
    }
    true
}

fn is_query_char(byte: u8) -> bool {
    is_pchar(byte) || matches!(byte, b'/' | b'?')
}

fn is_userinfo_char(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delim(byte) || byte == b':'
}

fn is_reg_name_char(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delim(byte)
}

fn is_pchar(byte: u8) -> bool {
    is_unreserved(byte) || is_sub_delim(byte) || matches!(byte, b':' | b'@')
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn is_sub_delim(byte: u8) -> bool {
    matches!(
        byte,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
    )
}

fn invalid_problem_type(expected_request_id: &str) -> Error {
    remote_protocol_error(
        "invalid_problem_type",
        "Problem Details type, category and code are inconsistent",
        expected_request_id,
    )
}

fn normalized_retry_hint(
    kind: ErrorKind,
    status: StatusCode,
    headers: &HeaderMap,
    now: SystemTime,
) -> RetryHint {
    if kind == ErrorKind::Application || !is_transient_status(status) {
        return RetryHint::Never;
    }
    parse_retry_after(headers, now).map_or(RetryHint::Retryable, RetryHint::After)
}

fn status_error(status: StatusCode, request_id: &str, headers: HeaderMap) -> Error {
    let kind = if status.is_client_error() {
        ErrorKind::Application
    } else {
        ErrorKind::Framework
    };
    let retry_hint = normalized_retry_hint(kind, status, &headers, SystemTime::now());
    Error::from_remote_parts(RemoteErrorParts {
        kind,
        category: category_from_status(status),
        code: ErrorCode::framework("remote_http_error"),
        message: status
            .canonical_reason()
            .unwrap_or("remote HTTP request failed")
            .to_owned(),
        status,
        request_id: request_id.to_owned(),
        details: None,
        retry_hint,
    })
    .with_headers(headers)
}

fn parse_retry_after(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    let mut values = headers.get_all(RETRY_AFTER).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let value = value.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|deadline| deadline.duration_since(now).unwrap_or(Duration::ZERO))
}

fn is_transient_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

fn framework_category_name(category: ErrorCategory) -> Option<&'static str> {
    match category {
        ErrorCategory::InvalidArgument => Some("invalid-argument"),
        ErrorCategory::NotFound => Some("not-found"),
        ErrorCategory::Conflict => Some("conflict"),
        ErrorCategory::Unauthenticated => Some("unauthenticated"),
        ErrorCategory::PermissionDenied => Some("permission-denied"),
        ErrorCategory::PayloadTooLarge => Some("payload-too-large"),
        ErrorCategory::ResourceExhausted => Some("resource-exhausted"),
        ErrorCategory::Unavailable => Some("unavailable"),
        ErrorCategory::DeadlineExceeded => Some("deadline-exceeded"),
        ErrorCategory::Cancelled => Some("cancelled"),
        ErrorCategory::Unimplemented => Some("unimplemented"),
        ErrorCategory::Internal => Some("internal"),
        ErrorCategory::DataLoss => Some("data-loss"),
        ErrorCategory::Unknown => None,
    }
}

fn framework_category_from_name(category: &str) -> Option<ErrorCategory> {
    match category {
        "invalid-argument" => Some(ErrorCategory::InvalidArgument),
        "not-found" => Some(ErrorCategory::NotFound),
        "conflict" => Some(ErrorCategory::Conflict),
        "unauthenticated" => Some(ErrorCategory::Unauthenticated),
        "permission-denied" => Some(ErrorCategory::PermissionDenied),
        "payload-too-large" => Some(ErrorCategory::PayloadTooLarge),
        "resource-exhausted" => Some(ErrorCategory::ResourceExhausted),
        "unavailable" => Some(ErrorCategory::Unavailable),
        "deadline-exceeded" => Some(ErrorCategory::DeadlineExceeded),
        "cancelled" => Some(ErrorCategory::Cancelled),
        "unimplemented" => Some(ErrorCategory::Unimplemented),
        "internal" => Some(ErrorCategory::Internal),
        "data-loss" => Some(ErrorCategory::DataLoss),
        _ => None,
    }
}

fn category_from_status(status: StatusCode) -> ErrorCategory {
    match status {
        StatusCode::BAD_REQUEST | StatusCode::METHOD_NOT_ALLOWED => ErrorCategory::InvalidArgument,
        StatusCode::UNAUTHORIZED => ErrorCategory::Unauthenticated,
        StatusCode::FORBIDDEN => ErrorCategory::PermissionDenied,
        StatusCode::NOT_FOUND => ErrorCategory::NotFound,
        StatusCode::CONFLICT => ErrorCategory::Conflict,
        StatusCode::PAYLOAD_TOO_LARGE => ErrorCategory::PayloadTooLarge,
        StatusCode::TOO_MANY_REQUESTS => ErrorCategory::ResourceExhausted,
        StatusCode::SERVICE_UNAVAILABLE => ErrorCategory::Unavailable,
        StatusCode::GATEWAY_TIMEOUT | StatusCode::REQUEST_TIMEOUT => {
            ErrorCategory::DeadlineExceeded
        }
        StatusCode::NOT_IMPLEMENTED => ErrorCategory::Unimplemented,
        StatusCode::INTERNAL_SERVER_ERROR => ErrorCategory::Internal,
        StatusCode::BAD_GATEWAY => ErrorCategory::DataLoss,
        status if status.as_u16() == 499 => ErrorCategory::Cancelled,
        _ => ErrorCategory::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderName, header::CONNECTION};
    use serde_json::{Value, json};

    fn problem_json(
        type_uri: &str,
        status: StatusCode,
        code: &str,
        request_id: &str,
        retryable: bool,
    ) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "type": type_uri,
            "title": status.canonical_reason().unwrap_or("Error"),
            "status": status.as_u16(),
            "detail": "remote detail",
            "code": code,
            "request_id": request_id,
            "retryable": retryable,
        }))
        .unwrap()
    }

    #[test]
    fn canonical_problem_validates_type_status_code_and_request_id() {
        let body = problem_json(
            "urn:fusen:error:unavailable:temporarily_unavailable",
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
            "request-1",
            true,
        );
        let error = decode_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "request-1",
            &body,
            HeaderMap::new(),
            true,
        );
        assert_eq!(error.origin(), ErrorOrigin::Remote);
        assert_eq!(error.category(), ErrorCategory::Unavailable);
        assert_eq!(error.retry_hint(), RetryHint::Retryable);

        for (invalid, expected_code) in [
            (
                problem_json(
                    "urn:fusen:error:unavailable:other_code",
                    StatusCode::SERVICE_UNAVAILABLE,
                    "temporarily_unavailable",
                    "request-1",
                    true,
                ),
                "invalid_problem_type",
            ),
            (
                problem_json(
                    "urn:fusen:error:unavailable:temporarily_unavailable",
                    StatusCode::BAD_REQUEST,
                    "temporarily_unavailable",
                    "request-1",
                    true,
                ),
                "invalid_problem_status",
            ),
            (
                problem_json(
                    "urn:fusen:error:unavailable:temporarily_unavailable",
                    StatusCode::SERVICE_UNAVAILABLE,
                    "temporarily_unavailable",
                    "different-request",
                    true,
                ),
                "invalid_problem_request_id",
            ),
        ] {
            let error = decode_problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "request-1",
                &invalid,
                HeaderMap::new(),
                true,
            );
            assert_eq!(error.origin(), ErrorOrigin::Remote);
            assert_eq!(error.category(), ErrorCategory::DataLoss);
            assert_eq!(error.status(), StatusCode::BAD_GATEWAY);
            assert_eq!(error.code().as_str(), expected_code);
        }
    }

    #[test]
    fn http_bindings_accept_external_problem_types_but_reserved_types_stay_strict() {
        for type_uri in [
            "about:blank",
            "https://errors.example.test/out-of-credit",
            "//errors.example.test/out-of-credit",
            "/problems/out-of-credit",
            "../problems/out-of-credit?source=upstream#version-1",
            "?problem=out-of-credit",
            "#out-of-credit",
            "foo://[v1.a]/problem",
        ] {
            let external = problem_json(
                type_uri,
                StatusCode::IM_A_TEAPOT,
                "teapot",
                "request-1",
                true,
            );
            for strict_controls in [false, true] {
                let error = decode_problem(
                    StatusCode::IM_A_TEAPOT,
                    "request-1",
                    &external,
                    HeaderMap::new(),
                    strict_controls,
                );
                assert_eq!(error.kind(), ErrorKind::Application, "type={type_uri}");
                assert_eq!(error.category(), ErrorCategory::Unknown, "type={type_uri}");
                assert_eq!(error.code().as_str(), "teapot", "type={type_uri}");
                assert_eq!(error.retry_hint(), RetryHint::Never, "type={type_uri}");
            }
        }

        let reserved = problem_json(
            "urn:fusen:error:future:teapot",
            StatusCode::IM_A_TEAPOT,
            "teapot",
            "request-1",
            false,
        );
        let error = decode_problem(
            StatusCode::IM_A_TEAPOT,
            "request-1",
            &reserved,
            HeaderMap::new(),
            true,
        );
        assert_eq!(error.code().as_str(), "invalid_problem_type");

        for type_uri in [
            "URN:fusen:error:application:teapot",
            "urn:FUSEN:error:application:teapot",
            "urn:fusen:ERROR:application:teapot",
        ] {
            let reserved = problem_json(
                type_uri,
                StatusCode::IM_A_TEAPOT,
                "teapot",
                "request-1",
                false,
            );
            let error = decode_problem(
                StatusCode::IM_A_TEAPOT,
                "request-1",
                &reserved,
                HeaderMap::new(),
                true,
            );
            assert_eq!(
                error.code().as_str(),
                "invalid_problem_type",
                "type={type_uri}"
            );
        }
    }

    #[test]
    fn malformed_external_problem_type_uri_references_are_rejected() {
        for type_uri in [
            "relative path",
            "/problems/%GG",
            "https://errors.example.test/%",
            "http://[::1",
            "https://errors.example.test/#one#two",
            ":relative",
            "?q=[]",
        ] {
            let body = problem_json(
                type_uri,
                StatusCode::IM_A_TEAPOT,
                "teapot",
                "request-1",
                false,
            );
            let error = decode_problem(
                StatusCode::IM_A_TEAPOT,
                "request-1",
                &body,
                HeaderMap::new(),
                false,
            );
            assert_eq!(
                error.code().as_str(),
                "invalid_problem_type",
                "type={type_uri}"
            );
        }
    }

    #[test]
    fn retry_hints_are_normalized_from_kind_status_and_headers() {
        let headers = HeaderMap::new();
        assert_eq!(
            normalized_retry_hint(
                ErrorKind::Framework,
                StatusCode::SERVICE_UNAVAILABLE,
                &headers,
                SystemTime::UNIX_EPOCH,
            ),
            RetryHint::Retryable
        );
        assert_eq!(
            normalized_retry_hint(
                ErrorKind::Framework,
                StatusCode::NOT_IMPLEMENTED,
                &headers,
                SystemTime::UNIX_EPOCH,
            ),
            RetryHint::Never
        );
        assert_eq!(
            normalized_retry_hint(
                ErrorKind::Application,
                StatusCode::SERVICE_UNAVAILABLE,
                &headers,
                SystemTime::UNIX_EPOCH,
            ),
            RetryHint::Never
        );
    }

    #[test]
    fn retry_after_accepts_delta_seconds_and_http_date() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("12"));
        assert_eq!(
            parse_retry_after(&headers, now),
            Some(Duration::from_secs(12))
        );
        headers.append(RETRY_AFTER, HeaderValue::from_static("13"));
        assert_eq!(parse_retry_after(&headers, now), None);

        let mut headers = HeaderMap::new();
        let future = httpdate::fmt_http_date(now + Duration::from_secs(23));
        headers.insert(RETRY_AFTER, HeaderValue::from_str(&future).unwrap());
        assert_eq!(
            parse_retry_after(&headers, now),
            Some(Duration::from_secs(23))
        );
        let past = httpdate::fmt_http_date(now - Duration::from_secs(1));
        headers.insert(RETRY_AFTER, HeaderValue::from_str(&past).unwrap());
        assert_eq!(parse_retry_after(&headers, now), Some(Duration::ZERO));
    }

    #[test]
    fn response_request_id_strictness_follows_negotiated_controls() {
        let headers = HeaderMap::new();
        assert!(validate_response_request_id(true, &headers, "request-1").is_err());
        validate_response_request_id(false, &headers, "request-1").unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(REQUEST_ID, HeaderValue::from_static("request-1"));
        validate_response_request_id(true, &headers, "request-1").unwrap();
        validate_response_request_id(false, &headers, "request-1").unwrap();
        assert!(validate_response_request_id(true, &headers, "request-2").is_err());
        validate_response_request_id(false, &headers, "request-2").unwrap();

        headers.append(REQUEST_ID, HeaderValue::from_static("not trusted"));
        assert!(validate_response_request_id(true, &headers, "request-1").is_err());
        validate_response_request_id(false, &headers, "request-1").unwrap();
    }

    #[test]
    fn problem_request_id_strictness_follows_negotiated_controls() {
        let mismatched = problem_json(
            "urn:fusen:error:unavailable:temporarily_unavailable",
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
            "different-request",
            true,
        );
        let relaxed = decode_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "request-1",
            &mismatched,
            HeaderMap::new(),
            false,
        );
        assert_eq!(relaxed.category(), ErrorCategory::Unavailable);
        assert_eq!(relaxed.request_id(), Some("different-request"));
        let strict = decode_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "request-1",
            &mismatched,
            HeaderMap::new(),
            true,
        );
        assert_eq!(strict.code().as_str(), "invalid_problem_request_id");

        let mut missing: Value = serde_json::from_slice(&mismatched).unwrap();
        missing.as_object_mut().unwrap().remove("request_id");
        let missing = serde_json::to_vec(&missing).unwrap();
        let relaxed = decode_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "request-1",
            &missing,
            HeaderMap::new(),
            false,
        );
        assert_eq!(relaxed.request_id(), Some("request-1"));
        let strict = decode_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "request-1",
            &missing,
            HeaderMap::new(),
            true,
        );
        assert_eq!(strict.code().as_str(), "invalid_problem_request_id");
    }

    #[test]
    fn malformed_problem_bodies_fall_back_to_remote_http_status() {
        for (status, kind, category, retry_hint) in [
            (
                StatusCode::NOT_FOUND,
                ErrorKind::Application,
                ErrorCategory::NotFound,
                RetryHint::Never,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorKind::Framework,
                ErrorCategory::Unavailable,
                RetryHint::Retryable,
            ),
        ] {
            let headers = HeaderMap::from_iter([(
                HeaderName::from_static("x-upstream"),
                HeaderValue::from_static("preserved"),
            )]);
            let error = decode_problem(status, "request-1", b"not-json", headers, false);
            assert_eq!(error.kind(), kind);
            assert_eq!(error.category(), category);
            assert_eq!(error.origin(), ErrorOrigin::Remote);
            assert_eq!(error.code().as_str(), "remote_http_error");
            assert_eq!(error.request_id(), Some("request-1"));
            assert_eq!(error.retry_hint(), retry_hint);
            assert_eq!(error.headers().get("x-upstream").unwrap(), "preserved");
        }

        let mismatched = problem_json(
            "about:blank",
            StatusCode::BAD_REQUEST,
            "upstream_error",
            "request-1",
            false,
        );
        let error = decode_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "request-1",
            &mismatched,
            HeaderMap::new(),
            false,
        );
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.category(), ErrorCategory::Unavailable);
        assert_eq!(error.code().as_str(), "remote_http_error");
    }

    #[test]
    fn head_errors_are_normalized_without_a_problem_body() {
        let headers = HeaderMap::from_iter([(RETRY_AFTER, HeaderValue::from_static("7"))]);
        let error = decode_head_error(StatusCode::SERVICE_UNAVAILABLE, "request-1", headers);
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.category(), ErrorCategory::Unavailable);
        assert_eq!(error.origin(), ErrorOrigin::Remote);
        assert_eq!(error.code().as_str(), "remote_head_error");
        assert_eq!(error.request_id(), Some("request-1"));
        assert_eq!(error.retry_hint(), RetryHint::After(Duration::from_secs(7)));
    }

    #[test]
    fn problem_encoding_emits_request_id_only_with_invocation_controls() {
        let error = Error::local(
            ErrorCategory::Unavailable,
            "temporarily_unavailable",
            "temporarily unavailable",
        )
        .unwrap();

        let controlled = encode_problem(&error, "request-1", None, true);
        assert_eq!(controlled.headers().get(REQUEST_ID).unwrap(), "request-1");
        let body = controlled.into_body().chunks[0].clone().unwrap();
        let problem: ProblemDetails = serde_json::from_slice(&body).unwrap();
        assert_eq!(problem.request_id.as_deref(), Some("request-1"));

        let plain = encode_problem(&error, "request-1", None, false);
        assert!(plain.headers().get(REQUEST_ID).is_none());
        let body = plain.into_body().chunks[0].clone().unwrap();
        let problem: ProblemDetails = serde_json::from_slice(&body).unwrap();
        assert!(problem.request_id.is_none());
    }

    #[test]
    fn encoding_does_not_relay_remote_headers_or_details() {
        let mut details = ErrorDetails::new();
        details.insert("field", Value::String("upstream".to_owned()));
        let mut headers = HeaderMap::new();
        headers.insert("x-upstream", HeaderValue::from_static("private"));
        headers.insert(CONNECTION, HeaderValue::from_static("close"));
        let remote = Error::from_remote_parts(RemoteErrorParts {
            kind: ErrorKind::Application,
            category: ErrorCategory::Conflict,
            code: ErrorCode::new("upstream_conflict").unwrap(),
            message: "conflict".to_owned(),
            status: StatusCode::CONFLICT,
            request_id: "upstream-request".to_owned(),
            details: Some(details),
            retry_hint: RetryHint::Never,
        })
        .with_headers(headers);

        let response = encode_problem(&remote, "current-request", None, true);
        assert!(response.headers().get("x-upstream").is_none());
        let body = response.into_body().chunks[0].clone().unwrap();
        let problem: ProblemDetails = serde_json::from_slice(&body).unwrap();
        assert_eq!(problem.request_id.as_deref(), Some("current-request"));
        assert!(problem.details.is_none());
    }

    #[test]
    fn encoding_only_emits_retry_after_for_local_framework_errors() {
        let remote = Error::from_remote_parts(RemoteErrorParts {
            kind: ErrorKind::Framework,
            category: ErrorCategory::Unavailable,
            code: ErrorCode::framework("upstream_unavailable"),
            message: "upstream unavailable".to_owned(),
            status: StatusCode::SERVICE_UNAVAILABLE,
            request_id: "upstream-request".to_owned(),
            details: None,
            retry_hint: RetryHint::After(Duration::from_secs(12)),
        });
        let response = encode_problem(&remote, "current-request", None, true);
        assert!(response.headers().get(RETRY_AFTER).is_none());

        let local = Error::local(
            ErrorCategory::Unavailable,
            "local_unavailable",
            "local service unavailable",
        )
        .unwrap()
        .with_retry_hint(RetryHint::After(Duration::from_millis(12_001)));
        let response = encode_problem(&local, "current-request", None, true);
        assert_eq!(response.headers().get(RETRY_AFTER).unwrap(), "13");
    }

    #[test]
    fn oversized_problem_uses_the_bounded_emergency_document() {
        let error = Error::application(
            ErrorCategory::InvalidArgument,
            "application_error",
            "x".repeat(EMERGENCY_PROBLEM_LIMIT * 2),
        )
        .unwrap();
        let (problem, _) = problem_from_error(&error, "request", None);
        let body = bounded_problem(&problem);
        assert!(body.len() <= EMERGENCY_PROBLEM_LIMIT);
        let problem: ProblemDetails = serde_json::from_slice(&body).unwrap();
        assert_eq!(problem.status, StatusCode::BAD_REQUEST.as_u16());
        assert_eq!(problem.code.as_ref().unwrap().as_str(), "application_error");
        assert_eq!(problem.request_id.as_deref(), Some("request"));
        assert!(!problem.retryable);
    }
}
