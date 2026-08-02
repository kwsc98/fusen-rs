use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::{Map, Value};
use std::{error::Error as StdError, fmt, sync::Arc, time::Duration};

/// A validated, stable machine-readable error code.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ErrorCode(String);

impl ErrorCode {
    /// Validates a lower-case snake-case error code.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidErrorCode> {
        let value = value.into();
        let bytes = value.as_bytes();
        let valid = !bytes.is_empty()
            && bytes.len() <= 64
            && bytes[0].is_ascii_lowercase()
            && !bytes.ends_with(b"_")
            && bytes.iter().enumerate().all(|(index, byte)| match byte {
                b'a'..=b'z' | b'0'..=b'9' => true,
                b'_' => index > 0 && bytes[index - 1] != b'_',
                _ => false,
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(InvalidErrorCode(value))
        }
    }

    pub(crate) fn framework(value: &'static str) -> Self {
        Self::new(value).expect("framework error codes must be valid snake_case")
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
#[error("invalid service invocation error code {0:?}")]
pub struct InvalidErrorCode(String);

/// Why an [`Error`] value could not be constructed.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorConstructionError {
    /// The machine-readable error code is invalid.
    #[error(transparent)]
    InvalidCode(#[from] InvalidErrorCode),
    /// The selected category has no canonical HTTP status.
    #[error("error category {0:?} has no canonical HTTP status")]
    NonCanonicalCategory(ErrorCategory),
    /// An application error used a status outside the 4xx/5xx ranges.
    #[error("application error status must be 4xx or 5xx, got {0}")]
    InvalidApplicationStatus(StatusCode),
}

/// Whether a service invocation failure was produced by application or framework code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Application code intentionally returned a public failure.
    Application,
    /// The runtime, an extension, transport, or peer framework failed.
    Framework,
}

/// Stable semantic category used for HTTP mapping and policy decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCategory {
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
    /// A request or response body exceeds its configured limit.
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
    /// The peer returned a valid status without a known semantic mapping.
    Unknown,
}

impl ErrorCategory {
    /// Returns this category's canonical HTTP status, when one exists.
    pub fn canonical_status(self) -> Option<StatusCode> {
        Some(match self {
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
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            Self::DataLoss => StatusCode::BAD_GATEWAY,
            Self::Unknown => return None,
        })
    }
}

/// Where a service invocation error originated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorOrigin {
    /// The local runtime, extension, or application produced the error.
    Local,
    /// A remote peer returned or caused the error.
    Remote,
}

/// A framework retry recommendation attached to a service invocation error.
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
/// the human-readable error message. Framework details are never written to the wire.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ErrorDetails(Map<String, Value>);

impl ErrorDetails {
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

/// A stable service invocation failure returned by generated clients and service implementations.
#[derive(Clone)]
#[non_exhaustive]
pub struct Error {
    inner: Arc<ErrorInner>,
}

#[derive(Clone)]
struct ErrorInner {
    kind: ErrorKind,
    category: ErrorCategory,
    code: ErrorCode,
    message: String,
    status: StatusCode,
    origin: ErrorOrigin,
    request_id: Option<String>,
    attempts: u8,
    headers: HeaderMap,
    details: Option<ErrorDetails>,
    retry_hint: RetryHint,
    source: Option<Arc<dyn StdError + Send + Sync + 'static>>,
}

pub(crate) struct RemoteErrorParts {
    pub(crate) kind: ErrorKind,
    pub(crate) category: ErrorCategory,
    pub(crate) code: ErrorCode,
    pub(crate) message: String,
    pub(crate) status: StatusCode,
    pub(crate) request_id: String,
    pub(crate) details: Option<ErrorDetails>,
    pub(crate) retry_hint: RetryHint,
}

impl Error {
    /// Creates a public application error with the category's canonical HTTP status.
    pub fn application(
        category: ErrorCategory,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, ErrorConstructionError> {
        let status = category
            .canonical_status()
            .ok_or(ErrorConstructionError::NonCanonicalCategory(category))?;
        Self::application_parts(category, status, code, message)
    }

    /// Creates a public application error with an explicit 4xx or 5xx HTTP status.
    pub fn application_status(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, ErrorConstructionError> {
        if !status.is_client_error() && !status.is_server_error() {
            return Err(ErrorConstructionError::InvalidApplicationStatus(status));
        }
        Self::application_parts(category_from_status(status), status, code, message)
    }

    fn application_parts(
        category: ErrorCategory,
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, ErrorConstructionError> {
        Ok(Self::from_inner(ErrorInner {
            kind: ErrorKind::Application,
            category,
            code: ErrorCode::new(code)?,
            message: message.into(),
            status,
            origin: ErrorOrigin::Local,
            request_id: None,
            attempts: 0,
            headers: HeaderMap::new(),
            details: None,
            retry_hint: RetryHint::Never,
            source: None,
        }))
    }

    /// Creates a typed, non-retryable local framework error.
    pub fn local(
        category: ErrorCategory,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, ErrorConstructionError> {
        let status = category
            .canonical_status()
            .ok_or(ErrorConstructionError::NonCanonicalCategory(category))?;
        Ok(Self::from_inner(ErrorInner {
            kind: ErrorKind::Framework,
            category,
            code: ErrorCode::new(code)?,
            message: message.into(),
            status,
            origin: ErrorOrigin::Local,
            request_id: None,
            attempts: 0,
            headers: HeaderMap::new(),
            details: None,
            retry_hint: RetryHint::Never,
            source: None,
        }))
    }

    pub(crate) fn framework(
        category: ErrorCategory,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        let status = category
            .canonical_status()
            .expect("framework errors require a canonical category");
        Self::from_inner(ErrorInner {
            kind: ErrorKind::Framework,
            category,
            code: ErrorCode::framework(code),
            message: message.into(),
            status,
            origin: ErrorOrigin::Local,
            request_id: None,
            attempts: 0,
            headers: HeaderMap::new(),
            details: None,
            retry_hint: RetryHint::Never,
            source: None,
        })
    }

    pub(crate) fn from_remote_parts(parts: RemoteErrorParts) -> Self {
        Self::from_inner(ErrorInner {
            kind: parts.kind,
            category: parts.category,
            code: parts.code,
            message: parts.message,
            status: parts.status,
            origin: ErrorOrigin::Remote,
            request_id: Some(parts.request_id),
            attempts: 0,
            headers: HeaderMap::new(),
            details: parts.details,
            retry_hint: if parts.kind == ErrorKind::Application {
                RetryHint::Never
            } else {
                parts.retry_hint
            },
            source: None,
        })
    }

    pub(crate) fn internal<E>(message: impl Into<String>, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::framework(ErrorCategory::Internal, "internal_error", message).with_source(source)
    }

    pub(crate) fn invalid_result<E>(message: impl Into<String>, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::framework(ErrorCategory::DataLoss, "invalid_result", message).with_source(source)
    }

    fn from_inner(inner: ErrorInner) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    fn inner_mut(&mut self) -> &mut ErrorInner {
        Arc::make_mut(&mut self.inner)
    }

    pub(crate) fn with_attempts(mut self, attempts: u8) -> Self {
        self.inner_mut().attempts = attempts;
        self
    }

    pub(crate) fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.inner_mut().request_id = Some(request_id.into());
        self
    }

    pub(crate) fn with_remote_origin(mut self) -> Self {
        self.inner_mut().origin = ErrorOrigin::Remote;
        self
    }

    pub(crate) fn with_local_origin(mut self) -> Self {
        self.inner_mut().origin = ErrorOrigin::Local;
        self
    }

    /// Returns whether application or framework code produced the error.
    pub fn kind(&self) -> ErrorKind {
        self.inner.kind
    }

    /// Returns the semantic category.
    pub fn category(&self) -> ErrorCategory {
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
    pub fn status(&self) -> StatusCode {
        self.inner.status
    }

    /// Returns where the error originated.
    pub fn origin(&self) -> ErrorOrigin {
        self.inner.origin
    }

    /// Returns the trusted logical invocation identifier, when assigned by the runtime.
    pub fn request_id(&self) -> Option<&str> {
        self.inner.request_id.as_deref()
    }

    /// Returns the number of physical attempts made by the logical invocation.
    pub fn attempts(&self) -> u8 {
        self.inner.attempts
    }

    /// Returns application response headers associated with the error.
    pub fn headers(&self) -> &HeaderMap {
        &self.inner.headers
    }

    /// Returns mutable application response headers associated with the error.
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.inner_mut().headers
    }

    /// Replaces the response headers associated with the error.
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.inner_mut().headers = headers;
        self
    }

    /// Returns structured public error details, when present.
    pub fn details(&self) -> Option<&ErrorDetails> {
        self.inner.details.as_ref()
    }

    /// Attaches structured public details to this error.
    ///
    /// Only locally-created application details are serialized to a remote caller.
    pub fn with_details(mut self, details: ErrorDetails) -> Self {
        self.inner_mut().details = Some(details);
        self
    }

    /// Returns the normalized framework retry recommendation.
    pub fn retry_hint(&self) -> RetryHint {
        self.inner.retry_hint
    }

    /// Sets the retry recommendation.
    ///
    /// Application errors remain non-retryable; replay eligibility is owned by the runtime.
    pub fn with_retry_hint(mut self, retry_hint: RetryHint) -> Self {
        if self.inner.kind != ErrorKind::Application {
            self.inner_mut().retry_hint = retry_hint;
        }
        self
    }

    /// Attaches an owned diagnostic source without exposing it on the wire.
    pub fn with_source<E>(mut self, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        self.inner_mut().source = Some(Arc::new(source));
        self
    }

    /// Returns the retained diagnostic source.
    pub fn source_ref(&self) -> Option<&(dyn StdError + Send + Sync + 'static)> {
        self.inner.source.as_deref()
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("kind", &self.inner.kind)
            .field("category", &self.inner.category)
            .field("code", &self.inner.code)
            .field("message", &self.inner.message)
            .field("status", &self.inner.status)
            .field("origin", &self.inner.origin)
            .field("request_id", &self.inner.request_id)
            .field("attempts", &self.inner.attempts)
            .field("retry_hint", &self.inner.retry_hint)
            .field("header_count", &self.inner.headers.len())
            .field("has_details", &self.inner.details.is_some())
            .field("has_source", &self.inner.source.is_some())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.inner.code, self.inner.message)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.inner
            .source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

pub(crate) fn category_from_status(status: StatusCode) -> ErrorCategory {
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
        StatusCode::BAD_GATEWAY => ErrorCategory::DataLoss,
        StatusCode::INTERNAL_SERVER_ERROR => ErrorCategory::Internal,
        status if status.as_u16() == 499 => ErrorCategory::Cancelled,
        _ => ErrorCategory::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderName, HeaderValue};

    #[test]
    fn invocation_error_keeps_a_pointer_sized_public_representation() {
        assert_eq!(std::mem::size_of::<Error>(), std::mem::size_of::<Arc<()>>());
    }

    #[test]
    fn error_codes_are_strict_snake_case() {
        for value in [
            "",
            "Bad",
            "bad-code",
            "_bad",
            "bad_",
            "bad__code",
            "2_bad",
            "bad code",
        ] {
            assert!(ErrorCode::new(value).is_err(), "{value:?}");
        }
        assert_eq!(
            ErrorCode::new("already_exists_2").unwrap().as_str(),
            "already_exists_2"
        );
        assert!(serde_json::from_str::<ErrorCode>(r#""invalid-code""#).is_err());
    }

    #[test]
    fn application_constructors_keep_kind_category_and_status_orthogonal() {
        let canonical =
            Error::application(ErrorCategory::Conflict, "duplicate", "already exists").unwrap();
        assert_eq!(canonical.kind(), ErrorKind::Application);
        assert_eq!(canonical.origin(), ErrorOrigin::Local);
        assert_eq!(canonical.status(), StatusCode::CONFLICT);
        assert_eq!(canonical.attempts(), 0);

        let custom = Error::application_status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_entity",
            "invalid entity",
        )
        .unwrap();
        assert_eq!(custom.category(), ErrorCategory::Unknown);
        assert_eq!(custom.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(matches!(
            Error::application(ErrorCategory::Unknown, "unknown", "unknown"),
            Err(ErrorConstructionError::NonCanonicalCategory(
                ErrorCategory::Unknown
            ))
        ));
    }

    #[test]
    fn application_errors_cannot_be_retryable() {
        let error = Error::application(ErrorCategory::Conflict, "duplicate", "already exists")
            .unwrap()
            .with_retry_hint(RetryHint::Retryable);
        assert_eq!(error.retry_hint(), RetryHint::Never);
    }

    #[test]
    fn cloning_is_copy_on_write_and_debug_redacts_values() {
        let mut details = ErrorDetails::new();
        details.insert("secret", Value::String("details secret".to_owned()));
        let mut original = Error::local(ErrorCategory::Internal, "failed", "safe message")
            .unwrap()
            .with_details(details)
            .with_source(std::io::Error::other("source secret"));
        original.headers_mut().insert(
            HeaderName::from_static("x-private"),
            HeaderValue::from_static("header secret"),
        );
        let mut cloned = original.clone();
        assert!(Arc::ptr_eq(&original.inner, &cloned.inner));

        cloned.headers_mut().clear();
        assert!(!Arc::ptr_eq(&original.inner, &cloned.inner));
        assert_eq!(original.headers().len(), 1);
        assert!(cloned.headers().is_empty());

        let debug = format!("{original:?}");
        assert!(!debug.contains("header secret"));
        assert!(!debug.contains("details secret"));
        assert!(!debug.contains("source secret"));
    }
}
