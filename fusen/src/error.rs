use http::StatusCode;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use thiserror::Error;

/// A validated application failure that can be returned over HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationError {
    status: StatusCode,
    code: String,
    message: String,
}

impl ApplicationError {
    /// Returns the validated 4xx or 5xx HTTP status.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the stable machine-readable application code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the public application error description.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl Display for ApplicationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "application error {}: {}",
            self.code, self.message
        )
    }
}

impl std::error::Error for ApplicationError {}

/// RFC 9457 problem details returned by the HTTP transport.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProblemDetails {
    /// Stable URI identifying the error category.
    #[serde(rename = "type")]
    pub type_uri: String,
    /// Human-readable HTTP status title.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Public detail that is safe to return to callers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Request path or other failing resource identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Stable machine-readable application code.
    pub code: String,
    /// Correlation identifier also written to server logs.
    pub request_id: String,
}

/// Stable framework and application errors.
#[derive(Error, Debug)]
pub enum FusenError {
    /// Request syntax or argument conversion failed.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// A peer returned a malformed or unsupported response.
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    /// No service method matches the request route.
    #[error("route not found: {0}")]
    RouteNotFound(String),
    /// The body exceeded the configured byte limit.
    #[error("payload exceeds {limit} bytes")]
    PayloadTooLarge {
        /// Configured maximum number of bytes.
        limit: usize,
    },
    /// The requested wire protocol is disabled or unknown.
    #[error("unsupported protocol: {0}")]
    UnsupportedProtocol(String),
    /// Discovery, load balancing, or concurrency could not provide capacity.
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
    /// A configured request deadline elapsed.
    #[error("request timed out: {0}")]
    Timeout(String),
    /// Stable error intentionally returned by application code.
    #[error("{0}")]
    Application(ApplicationError),
    /// RFC 9457 error returned by a remote service.
    #[error("remote error {0:?}")]
    Remote(Box<ProblemDetails>),
    /// Internal failure whose source is logged but hidden from callers.
    #[error("{message}: {source}")]
    Internal {
        /// Non-sensitive framework context written to server logs.
        message: &'static str,
        #[source]
        /// Thread-safe source retained for diagnostics.
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}

impl FusenError {
    /// Creates a validated application error with a 4xx or 5xx status.
    pub fn application(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, Self> {
        if !status.is_client_error() && !status.is_server_error() {
            return Err(Self::InvalidRequest(format!(
                "application error status must be 4xx or 5xx, got {status}"
            )));
        }
        Ok(Self::Application(ApplicationError {
            status,
            code: code.into(),
            message: message.into(),
        }))
    }

    /// Wraps a thread-safe source as a non-public internal error.
    pub fn internal<E>(message: &'static str, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Internal {
            message,
            source: Box::new(source),
        }
    }

    /// Returns the HTTP status associated with this error.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            Self::InvalidResponse(_) => StatusCode::BAD_GATEWAY,
            Self::RouteNotFound(_) => StatusCode::NOT_FOUND,
            Self::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedProtocol(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
            Self::Application(error) => error.status(),
            Self::Remote(problem) => {
                StatusCode::from_u16(problem.status).unwrap_or(StatusCode::BAD_GATEWAY)
            }
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Returns the stable machine-readable error code.
    pub fn code(&self) -> &str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::InvalidResponse(_) => "invalid_response",
            Self::RouteNotFound(_) => "route_not_found",
            Self::PayloadTooLarge { .. } => "payload_too_large",
            Self::UnsupportedProtocol(_) => "unsupported_protocol",
            Self::ServiceUnavailable(_) => "service_unavailable",
            Self::Timeout(_) => "timeout",
            Self::Application(error) => error.code(),
            Self::Remote(problem) => &problem.code,
            Self::Internal { .. } => "internal_error",
        }
    }

    /// Converts the error into a public RFC 9457 representation.
    pub fn problem_details(
        &self,
        request_id: impl Into<String>,
        instance: Option<String>,
    ) -> ProblemDetails {
        let status = self.status();
        let code = self.code().to_owned();
        let detail = match self {
            Self::Internal { .. } => Some("Internal server error".to_owned()),
            Self::Remote(problem) => problem.detail.clone(),
            _ => Some(self.to_string()),
        };
        ProblemDetails {
            type_uri: format!("https://fusen.rs/problems/{code}"),
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            status: status.as_u16(),
            detail,
            instance,
            code,
            request_id: request_id.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_details_are_hidden() {
        let error = FusenError::internal("database failed", std::io::Error::other("secret"));
        let problem = error.problem_details("request-1", None);
        assert_eq!(problem.status, 500);
        assert_eq!(problem.detail.as_deref(), Some("Internal server error"));
        assert!(!problem.detail.unwrap().contains("secret"));
    }

    #[test]
    fn application_errors_reject_non_error_statuses() {
        for status in [StatusCode::CONTINUE, StatusCode::OK, StatusCode::FOUND] {
            assert!(FusenError::application(status, "invalid", "invalid").is_err());
        }
    }

    #[test]
    fn application_errors_expose_validated_fields() {
        let error =
            FusenError::application(StatusCode::CONFLICT, "duplicate", "already exists").unwrap();
        let FusenError::Application(application) = error else {
            panic!("expected an application error");
        };
        assert_eq!(application.status(), StatusCode::CONFLICT);
        assert_eq!(application.code(), "duplicate");
        assert_eq!(application.message(), "already exists");
    }
}
