use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    #[error("application error {code}: {message}")]
    Application {
        /// HTTP status returned to the caller.
        status: u16,
        /// Stable machine-readable application code.
        code: String,
        /// Public application error description.
        message: String,
    },
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
    pub fn status(&self) -> u16 {
        match self {
            Self::InvalidRequest(_) => 400,
            Self::RouteNotFound(_) => 404,
            Self::PayloadTooLarge { .. } => 413,
            Self::UnsupportedProtocol(_) => 415,
            Self::ServiceUnavailable(_) => 503,
            Self::Timeout(_) => 504,
            Self::Application { status, .. } => *status,
            Self::Remote(problem) => problem.status,
            Self::Internal { .. } => 500,
        }
    }

    /// Returns the stable machine-readable error code.
    pub fn code(&self) -> &str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::RouteNotFound(_) => "route_not_found",
            Self::PayloadTooLarge { .. } => "payload_too_large",
            Self::UnsupportedProtocol(_) => "unsupported_protocol",
            Self::ServiceUnavailable(_) => "service_unavailable",
            Self::Timeout(_) => "timeout",
            Self::Application { code, .. } => code,
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
            title: http::StatusCode::from_u16(status)
                .ok()
                .and_then(|value| value.canonical_reason())
                .unwrap_or("Error")
                .to_owned(),
            status,
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
}
