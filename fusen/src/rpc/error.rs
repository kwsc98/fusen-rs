use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Map, Value};
use std::{error::Error, fmt, sync::Arc, time::Duration};

/// A validated, stable machine-readable error code.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ErrorCode(String);

impl ErrorCode {
    /// Validates a lower-case snake-case error code.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidErrorCode> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value.bytes().enumerate().all(|(index, byte)| match byte {
                b'a'..=b'z' => true,
                b'0'..=b'9' | b'_' => index > 0,
                _ => false,
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(InvalidErrorCode(value))
        }
    }

    pub(crate) fn framework(value: &'static str) -> Self {
        debug_assert!(Self::new(value).is_ok());
        Self(value.to_owned())
    }

    /// Returns the wire representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// An invalid machine-readable error code.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
#[error("invalid RPC error code {0:?}")]
pub struct InvalidErrorCode(String);

/// Stable semantic category used for HTTP mapping and policy decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RpcCategory {
    /// Request syntax or values are invalid.
    InvalidArgument,
    /// The requested route or resource does not exist.
    NotFound,
    /// Existing state conflicts with the request.
    Conflict,
    /// Authentication is required or invalid.
    Unauthenticated,
    /// The caller is authenticated but not authorized.
    PermissionDenied,
    /// A request body exceeds its configured limit.
    PayloadTooLarge,
    /// Admission or another bounded resource is exhausted.
    ResourceExhausted,
    /// No healthy service or transport is currently available.
    Unavailable,
    /// The end-to-end invocation deadline elapsed.
    DeadlineExceeded,
    /// The caller cancelled the invocation.
    Cancelled,
    /// The requested capability is not implemented or enabled.
    Unimplemented,
    /// A private framework or application failure occurred.
    Internal,
    /// A peer produced corrupt or malformed data.
    DataLoss,
    /// A stable error intentionally returned by application code.
    Application,
}

impl RpcCategory {
    /// Returns the canonical HTTP status for this category.
    pub fn status(self) -> StatusCode {
        match self {
            Self::InvalidArgument => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict => StatusCode::CONFLICT,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::PermissionDenied => StatusCode::FORBIDDEN,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
            Self::Cancelled => StatusCode::from_u16(499).expect("499 is a valid HTTP status"),
            Self::Unimplemented => StatusCode::NOT_IMPLEMENTED,
            Self::Internal | Self::Application => StatusCode::INTERNAL_SERVER_ERROR,
            Self::DataLoss => StatusCode::BAD_GATEWAY,
        }
    }
}

/// Where an RPC error originated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RpcOrigin {
    /// The local runtime rejected or failed the call.
    Local,
    /// A remote peer returned the error.
    Remote,
    /// User service code intentionally returned the error.
    Application,
}

/// A framework retry recommendation attached to an RPC error.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RetryHint {
    /// The runtime must not retry based on this error.
    #[default]
    Never,
    /// A replayable invocation may be retried according to its retry policy.
    Retryable,
    /// A replayable invocation may be retried after at least this duration.
    After(Duration),
}

impl RetryHint {
    /// Returns whether this hint permits retrying a replayable invocation.
    pub const fn is_retryable(self) -> bool {
        !matches!(self, Self::Never)
    }

    /// Returns the minimum retry delay when one was supplied.
    pub const fn retry_after(self) -> Option<Duration> {
        match self {
            Self::After(duration) => Some(duration),
            Self::Never | Self::Retryable => None,
        }
    }
}

/// Structured, public application-error metadata.
///
/// Details are represented as a JSON object so callers can inspect stable fields without parsing
/// the human-readable error message. Local framework details are never written to the wire.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RpcErrorDetails(Map<String, Value>);

impl RpcErrorDetails {
    /// Creates an empty details object.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates details from an existing JSON object.
    pub fn from_map(fields: Map<String, Value>) -> Self {
        Self(fields)
    }

    /// Returns one structured field.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    /// Inserts or replaces one structured field.
    pub fn insert(&mut self, name: impl Into<String>, value: Value) -> Option<Value> {
        self.0.insert(name.into(), value)
    }

    /// Iterates over the structured fields.
    pub fn iter(&self) -> serde_json::map::Iter<'_> {
        self.0.iter()
    }

    /// Returns whether the details object contains no fields.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the number of structured fields.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Consumes the wrapper and returns the JSON object.
    pub fn into_map(self) -> Map<String, Value> {
        self.0
    }
}

/// Private RFC 9457 wire document used by both supported JSON protocols.
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
    code: ErrorCode,
    request_id: String,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<RpcErrorDetails>,
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
        details: Option<RpcErrorDetails>,
    ) -> Self {
        Self {
            type_uri: type_uri.into(),
            title: title.into(),
            status,
            detail,
            instance,
            code,
            request_id: request_id.into(),
            retryable,
            details,
        }
    }

    pub(crate) fn status(&self) -> u16 {
        self.status
    }

    #[cfg(test)]
    pub(crate) fn code(&self) -> &ErrorCode {
        &self.code
    }

    #[cfg(test)]
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    #[cfg(test)]
    pub(crate) fn retryable(&self) -> bool {
        self.retryable
    }

    pub(crate) fn without_optional_fields(&self) -> Self {
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
}

/// A stable RPC failure returned by generated clients and service implementations.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RpcError {
    inner: Box<RpcErrorInner>,
}

#[derive(Clone, Debug)]
struct RpcErrorInner {
    category: RpcCategory,
    code: ErrorCode,
    message: String,
    status: StatusCode,
    origin: RpcOrigin,
    attempts: u8,
    headers: HeaderMap,
    details: Option<RpcErrorDetails>,
    retry_hint: RetryHint,
    source: Option<Arc<dyn Error + Send + Sync + 'static>>,
}

impl RpcError {
    /// Creates a public application error. Application errors are never automatically retryable.
    pub fn application(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, InvalidErrorCode> {
        if !status.is_client_error() && !status.is_server_error() {
            return Err(InvalidErrorCode(format!("HTTP status {status}")));
        }
        Ok(Self::from_inner(RpcErrorInner {
            category: RpcCategory::Application,
            code: ErrorCode::new(code)?,
            message: message.into(),
            status,
            origin: RpcOrigin::Application,
            attempts: 1,
            headers: HeaderMap::new(),
            details: None,
            retry_hint: RetryHint::Never,
            source: None,
        }))
    }

    /// Creates a typed, non-retryable local error.
    pub fn new(
        category: RpcCategory,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, InvalidErrorCode> {
        let origin = if category == RpcCategory::Application {
            RpcOrigin::Application
        } else {
            RpcOrigin::Local
        };
        Ok(Self::from_inner(RpcErrorInner {
            category,
            code: ErrorCode::new(code)?,
            message: message.into(),
            status: category.status(),
            origin,
            attempts: 1,
            headers: HeaderMap::new(),
            details: None,
            retry_hint: RetryHint::Never,
            source: None,
        }))
    }

    pub(crate) fn framework(
        category: RpcCategory,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::from_inner(RpcErrorInner {
            category,
            code: ErrorCode::framework(code),
            message: message.into(),
            status: category.status(),
            origin: RpcOrigin::Local,
            attempts: 1,
            headers: HeaderMap::new(),
            details: None,
            retry_hint: RetryHint::Never,
            source: None,
        })
    }

    pub(crate) fn internal<E>(message: impl Into<String>, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::framework(RpcCategory::Internal, "internal_error", message).with_source(source)
    }

    pub(crate) fn invalid_result<E>(message: impl Into<String>, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self::framework(RpcCategory::DataLoss, "invalid_result", message).with_source(source)
    }

    pub(crate) fn from_remote(problem: ProblemDetails) -> Self {
        let status = StatusCode::from_u16(problem.status).unwrap_or(StatusCode::BAD_GATEWAY);
        let category = category_from_type_uri(&problem.type_uri)
            .unwrap_or_else(|| category_from_status(status));
        let origin = if category == RpcCategory::Application {
            RpcOrigin::Application
        } else {
            RpcOrigin::Remote
        };
        Self::from_inner(RpcErrorInner {
            category,
            code: problem.code,
            message: problem
                .detail
                .unwrap_or_else(|| "remote service returned an error".to_owned()),
            status,
            origin,
            attempts: 1,
            headers: HeaderMap::new(),
            details: problem.details,
            retry_hint: if problem.retryable && origin != RpcOrigin::Application {
                RetryHint::Retryable
            } else {
                RetryHint::Never
            },
            source: None,
        })
    }

    pub(crate) fn from_remote_head(status: StatusCode) -> Self {
        let category = category_from_status(status);
        Self::from_inner(RpcErrorInner {
            category,
            code: ErrorCode::framework("remote_head_error"),
            message: status
                .canonical_reason()
                .unwrap_or("remote HEAD request failed")
                .to_owned(),
            status,
            origin: RpcOrigin::Remote,
            attempts: 1,
            headers: HeaderMap::new(),
            details: None,
            retry_hint: RetryHint::Never,
            source: None,
        })
    }

    fn from_inner(inner: RpcErrorInner) -> Self {
        Self {
            inner: Box::new(inner),
        }
    }

    pub(crate) fn with_attempts(mut self, attempts: u8) -> Self {
        self.inner.attempts = attempts.max(1);
        self
    }

    /// Returns the semantic category.
    pub const fn category(&self) -> RpcCategory {
        self.inner.category
    }

    /// Returns the stable machine-readable code.
    pub fn code(&self) -> &ErrorCode {
        &self.inner.code
    }

    /// Returns the safe human-readable message.
    pub fn message(&self) -> &str {
        &self.inner.message
    }

    /// Returns the mapped HTTP status.
    pub const fn status(&self) -> StatusCode {
        self.inner.status
    }

    /// Returns where the error originated.
    pub const fn origin(&self) -> RpcOrigin {
        self.inner.origin
    }

    /// Returns the number of physical attempts made by the logical invocation.
    pub const fn attempts(&self) -> u8 {
        self.inner.attempts
    }

    /// Returns application response headers associated with the error.
    pub const fn headers(&self) -> &HeaderMap {
        &self.inner.headers
    }

    /// Returns mutable application response headers associated with the error.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.inner.headers
    }

    /// Replaces the response headers associated with the error.
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.inner.headers = headers;
        self
    }

    /// Returns structured public error details, when present.
    pub const fn details(&self) -> Option<&RpcErrorDetails> {
        self.inner.details.as_ref()
    }

    /// Attaches structured public details to this error.
    ///
    /// Only application-origin details are serialized to a remote caller. Framework and local
    /// diagnostic details remain local even when attached for in-process inspection.
    pub fn with_details(mut self, details: RpcErrorDetails) -> Self {
        self.inner.details = Some(details);
        self
    }

    /// Returns the remote or framework retry recommendation.
    pub const fn retry_hint(&self) -> RetryHint {
        self.inner.retry_hint
    }

    /// Sets the retry recommendation.
    ///
    /// Application errors remain non-retryable; replay eligibility is owned by the runtime and
    /// derived from the method's standard HTTP mapping.
    pub fn with_retry_hint(mut self, retry_hint: RetryHint) -> Self {
        if self.inner.origin != RpcOrigin::Application {
            self.inner.retry_hint = retry_hint;
        }
        self
    }

    /// Attaches an owned diagnostic source without exposing it on the wire.
    pub fn with_source<E>(mut self, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        self.inner.source = Some(Arc::new(source));
        self
    }

    /// Returns the retained diagnostic source.
    pub fn source_ref(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        self.inner.source.as_deref()
    }

    pub(crate) fn problem_details(
        &self,
        request_id: impl Into<String>,
        instance: Option<String>,
    ) -> ProblemDetails {
        let detail = if self.inner.category == RpcCategory::Internal {
            Some("Internal server error".to_owned())
        } else {
            Some(self.inner.message.clone())
        };
        ProblemDetails {
            type_uri: format!(
                "urn:fusen:error:{}:{}",
                category_name(self.inner.category),
                self.inner.code
            ),
            title: self
                .inner
                .status
                .canonical_reason()
                .unwrap_or("RPC Error")
                .to_owned(),
            status: self.inner.status.as_u16(),
            detail,
            instance,
            code: self.inner.code.clone(),
            request_id: request_id.into(),
            retryable: self.inner.retry_hint.is_retryable()
                && self.inner.origin != RpcOrigin::Application,
            details: (self.inner.origin == RpcOrigin::Application)
                .then(|| self.inner.details.clone())
                .flatten(),
        }
    }
}

fn category_name(category: RpcCategory) -> &'static str {
    match category {
        RpcCategory::InvalidArgument => "invalid-argument",
        RpcCategory::NotFound => "not-found",
        RpcCategory::Conflict => "conflict",
        RpcCategory::Unauthenticated => "unauthenticated",
        RpcCategory::PermissionDenied => "permission-denied",
        RpcCategory::PayloadTooLarge => "payload-too-large",
        RpcCategory::ResourceExhausted => "resource-exhausted",
        RpcCategory::Unavailable => "unavailable",
        RpcCategory::DeadlineExceeded => "deadline-exceeded",
        RpcCategory::Cancelled => "cancelled",
        RpcCategory::Unimplemented => "unimplemented",
        RpcCategory::Internal => "internal",
        RpcCategory::DataLoss => "data-loss",
        RpcCategory::Application => "application",
    }
}

fn category_from_type_uri(type_uri: &str) -> Option<RpcCategory> {
    let category = type_uri
        .strip_prefix("urn:fusen:error:")?
        .split(':')
        .next()?;
    match category {
        "invalid-argument" => Some(RpcCategory::InvalidArgument),
        "not-found" => Some(RpcCategory::NotFound),
        "conflict" => Some(RpcCategory::Conflict),
        "unauthenticated" => Some(RpcCategory::Unauthenticated),
        "permission-denied" => Some(RpcCategory::PermissionDenied),
        "payload-too-large" => Some(RpcCategory::PayloadTooLarge),
        "resource-exhausted" => Some(RpcCategory::ResourceExhausted),
        "unavailable" => Some(RpcCategory::Unavailable),
        "deadline-exceeded" => Some(RpcCategory::DeadlineExceeded),
        "cancelled" => Some(RpcCategory::Cancelled),
        "unimplemented" => Some(RpcCategory::Unimplemented),
        "internal" => Some(RpcCategory::Internal),
        "data-loss" => Some(RpcCategory::DataLoss),
        "application" => Some(RpcCategory::Application),
        _ => None,
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.inner.code, self.inner.message)
    }
}

impl Error for RpcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.inner
            .source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

fn category_from_status(status: StatusCode) -> RpcCategory {
    match status {
        StatusCode::BAD_REQUEST | StatusCode::METHOD_NOT_ALLOWED => RpcCategory::InvalidArgument,
        StatusCode::UNAUTHORIZED => RpcCategory::Unauthenticated,
        StatusCode::FORBIDDEN => RpcCategory::PermissionDenied,
        StatusCode::NOT_FOUND => RpcCategory::NotFound,
        StatusCode::CONFLICT => RpcCategory::Conflict,
        StatusCode::PAYLOAD_TOO_LARGE => RpcCategory::PayloadTooLarge,
        StatusCode::TOO_MANY_REQUESTS => RpcCategory::ResourceExhausted,
        StatusCode::SERVICE_UNAVAILABLE => RpcCategory::Unavailable,
        StatusCode::GATEWAY_TIMEOUT | StatusCode::REQUEST_TIMEOUT => RpcCategory::DeadlineExceeded,
        StatusCode::NOT_IMPLEMENTED => RpcCategory::Unimplemented,
        StatusCode::BAD_GATEWAY => RpcCategory::DataLoss,
        _ => RpcCategory::Application,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_error_keeps_a_pointer_sized_public_result_representation() {
        assert_eq!(
            std::mem::size_of::<RpcError>(),
            std::mem::size_of::<Box<()>>()
        );
    }

    #[test]
    fn error_codes_are_strict_snake_case() {
        for value in ["", "Bad", "bad-code", "_bad", "bad code"] {
            assert!(ErrorCode::new(value).is_err(), "{value:?}");
        }
        assert_eq!(
            ErrorCode::new("already_exists_2").unwrap().as_str(),
            "already_exists_2"
        );
        assert!(serde_json::from_str::<ErrorCode>(r#""invalid-code""#).is_err());
    }

    #[test]
    fn internal_sources_never_enter_problem_details() {
        let mut details = RpcErrorDetails::new();
        details.insert("query", Value::String("secret query".to_owned()));
        let error = RpcError::internal(
            "database operation failed",
            std::io::Error::other("secret source"),
        )
        .with_details(details);
        let problem = error.problem_details("request-1", Some("/rpc".into()));
        assert_eq!(problem.code.as_str(), "internal_error");
        assert_eq!(problem.detail.as_deref(), Some("Internal server error"));
        let encoded = serde_json::to_string(&problem).unwrap();
        assert!(!encoded.contains("secret source"));
        assert!(!encoded.contains("secret query"));
        assert!(error.source_ref().is_some());
    }

    #[test]
    fn application_details_round_trip_as_structured_data() {
        let mut details = RpcErrorDetails::new();
        details.insert("field", Value::String("email".to_owned()));
        details.insert("constraint", Value::String("unique".to_owned()));
        let error = RpcError::application(StatusCode::CONFLICT, "duplicate", "already exists")
            .unwrap()
            .with_details(details.clone());

        assert_eq!(error.details(), Some(&details));
        let remote = RpcError::from_remote(error.problem_details("request-2", None));
        assert_eq!(remote.details(), Some(&details));
    }

    #[test]
    fn application_errors_cannot_be_retryable() {
        let error = RpcError::application(StatusCode::CONFLICT, "duplicate", "already exists")
            .unwrap()
            .with_retry_hint(RetryHint::Retryable);
        assert!(!error.retry_hint().is_retryable());

        let typed = RpcError::new(RpcCategory::Application, "domain_error", "failed").unwrap();
        assert_eq!(typed.origin(), RpcOrigin::Application);
        assert!(!typed.retry_hint().is_retryable());

        let mut malicious = typed.problem_details("request-1", None);
        malicious.retryable = true;
        let remote = RpcError::from_remote(malicious);
        assert_eq!(remote.origin(), RpcOrigin::Application);
        assert!(!remote.retry_hint().is_retryable());
    }

    #[test]
    fn problem_type_round_trips_internal_and_application_categories() {
        let internal = RpcError::framework(RpcCategory::Internal, "service_panic", "hidden");
        let internal = RpcError::from_remote(internal.problem_details("request-1", None));
        assert_eq!(internal.category(), RpcCategory::Internal);
        assert_eq!(internal.origin(), RpcOrigin::Remote);

        let application = RpcError::application(
            StatusCode::INTERNAL_SERVER_ERROR,
            "domain_failure",
            "public",
        )
        .unwrap();
        let application = RpcError::from_remote(application.problem_details("request-2", None));
        assert_eq!(application.category(), RpcCategory::Application);
        assert_eq!(application.origin(), RpcOrigin::Application);
        assert!(!application.retry_hint().is_retryable());
    }
}
