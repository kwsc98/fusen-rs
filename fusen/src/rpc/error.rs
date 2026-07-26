use http::StatusCode;
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{error::Error, fmt, sync::Arc};

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

/// RFC 9457 problem details used by both supported JSON protocols.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProblemDetails {
    /// Stable problem type URI.
    #[serde(rename = "type")]
    pub type_uri: String,
    /// Human-readable status title.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Public detail safe to return to callers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Request path or other failing resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Stable machine-readable code.
    pub code: ErrorCode,
    /// Correlation identifier assigned by the server.
    pub request_id: String,
    /// Whether a framework client may consider retrying an explicitly idempotent method.
    pub retryable: bool,
}

/// A stable RPC failure returned by generated clients and service implementations.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RpcError {
    category: RpcCategory,
    code: ErrorCode,
    message: String,
    status: StatusCode,
    origin: RpcOrigin,
    attempts: u8,
    retryable: bool,
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
        Ok(Self {
            category: RpcCategory::Application,
            code: ErrorCode::new(code)?,
            message: message.into(),
            status,
            origin: RpcOrigin::Application,
            attempts: 1,
            retryable: false,
            source: None,
        })
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
        Ok(Self {
            category,
            code: ErrorCode::new(code)?,
            message: message.into(),
            status: category.status(),
            origin,
            attempts: 1,
            retryable: false,
            source: None,
        })
    }

    pub(crate) fn framework(
        category: RpcCategory,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            category,
            code: ErrorCode::framework(code),
            message: message.into(),
            status: category.status(),
            origin: RpcOrigin::Local,
            attempts: 1,
            retryable: false,
            source: None,
        }
    }

    pub(crate) fn internal<E>(message: impl Into<String>, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            source: Some(Arc::new(source)),
            ..Self::framework(RpcCategory::Internal, "internal_error", message)
        }
    }

    pub(crate) fn invalid_result<E>(message: impl Into<String>, source: E) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            source: Some(Arc::new(source)),
            ..Self::framework(RpcCategory::DataLoss, "invalid_result", message)
        }
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
        Self {
            category,
            code: problem.code,
            message: problem
                .detail
                .unwrap_or_else(|| "remote service returned an error".to_owned()),
            status,
            origin,
            attempts: 1,
            retryable: problem.retryable && origin != RpcOrigin::Application,
            source: None,
        }
    }

    pub(crate) fn from_remote_head(status: StatusCode) -> Self {
        let category = category_from_status(status);
        Self {
            category,
            code: ErrorCode::framework("remote_head_error"),
            message: status
                .canonical_reason()
                .unwrap_or("remote HEAD request failed")
                .to_owned(),
            status,
            origin: RpcOrigin::Remote,
            attempts: 1,
            retryable: false,
            source: None,
        }
    }

    pub(crate) fn with_attempts(mut self, attempts: u8) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    pub(crate) fn mark_retryable(mut self) -> Self {
        if self.origin != RpcOrigin::Application {
            self.retryable = true;
        }
        self
    }

    /// Returns the semantic category.
    pub const fn category(&self) -> RpcCategory {
        self.category
    }

    /// Returns the stable machine-readable code.
    pub fn code(&self) -> &ErrorCode {
        &self.code
    }

    /// Returns the safe human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the mapped HTTP status.
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns where the error originated.
    pub const fn origin(&self) -> RpcOrigin {
        self.origin
    }

    /// Returns the number of physical attempts made by the logical invocation.
    pub const fn attempts(&self) -> u8 {
        self.attempts
    }

    /// Returns the remote or framework retry hint.
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub(crate) fn problem_details(
        &self,
        request_id: impl Into<String>,
        instance: Option<String>,
    ) -> ProblemDetails {
        let detail = if self.category == RpcCategory::Internal {
            Some("Internal server error".to_owned())
        } else {
            Some(self.message.clone())
        };
        ProblemDetails {
            type_uri: format!(
                "urn:fusen:error:{}:{}",
                category_name(self.category),
                self.code
            ),
            title: self
                .status
                .canonical_reason()
                .unwrap_or("RPC Error")
                .to_owned(),
            status: self.status.as_u16(),
            detail,
            instance,
            code: self.code.clone(),
            request_id: request_id.into(),
            retryable: self.retryable && self.origin != RpcOrigin::Application,
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
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for RpcError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
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
        let error =
            RpcError::internal("database operation failed", std::io::Error::other("secret"));
        let problem = error.problem_details("request-1", Some("/rpc".into()));
        assert_eq!(problem.code.as_str(), "internal_error");
        assert_eq!(problem.detail.as_deref(), Some("Internal server error"));
        assert!(!serde_json::to_string(&problem).unwrap().contains("secret"));
    }

    #[test]
    fn application_errors_cannot_be_retryable() {
        let error = RpcError::application(StatusCode::CONFLICT, "duplicate", "already exists")
            .unwrap()
            .mark_retryable();
        assert!(!error.retryable());

        let typed = RpcError::new(RpcCategory::Application, "domain_error", "failed").unwrap();
        assert_eq!(typed.origin(), RpcOrigin::Application);
        assert!(!typed.retryable());

        let mut malicious = typed.problem_details("request-1", None);
        malicious.retryable = true;
        let remote = RpcError::from_remote(malicious);
        assert_eq!(remote.origin(), RpcOrigin::Application);
        assert!(!remote.retryable());
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
        assert!(!application.retryable());
    }
}
