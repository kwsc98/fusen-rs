use crate::{Error, ErrorCategory, ErrorKind, ErrorOrigin};

use super::breaker::FailureClass;

/// One invocation error paired with the policy classification chosen at its failure boundary.
pub(crate) struct ClassifiedError {
    error: Error,
    class: FailureClass,
}

impl ClassifiedError {
    pub(crate) fn new(error: Error, class: FailureClass) -> Self {
        Self { error, class }
    }

    pub(crate) fn classify(error: Error) -> Self {
        let class = classify_error(&error);
        Self { error, class }
    }

    pub(crate) const fn class(&self) -> FailureClass {
        self.class
    }

    pub(crate) fn into_error(self) -> Error {
        self.error
    }
}

impl From<Error> for ClassifiedError {
    fn from(error: Error) -> Self {
        Self::classify(error)
    }
}

pub(crate) fn classify_error(error: &Error) -> FailureClass {
    if error.kind() == ErrorKind::Framework
        && error.origin() == ErrorOrigin::Remote
        && error.category() == ErrorCategory::DataLoss
    {
        return FailureClass::Protocol;
    }

    if error.kind() == ErrorKind::Application {
        return match (error.origin(), error.status().is_server_error()) {
            (ErrorOrigin::Remote, true) => FailureClass::RemoteFailure,
            (ErrorOrigin::Remote, false) => FailureClass::Application,
            (ErrorOrigin::Local, _) => FailureClass::InvalidRequest,
        };
    }

    if error.origin() == ErrorOrigin::Local {
        if error.retry_hint().is_retryable() {
            return FailureClass::Transport;
        }
        return match error.category() {
            ErrorCategory::DeadlineExceeded => FailureClass::Timeout,
            ErrorCategory::Cancelled => FailureClass::Cancelled,
            ErrorCategory::DataLoss => FailureClass::Protocol,
            ErrorCategory::PayloadTooLarge
            | ErrorCategory::ResourceExhausted
            | ErrorCategory::Unavailable => FailureClass::LocalRejection,
            _ => FailureClass::InvalidRequest,
        };
    }

    let retryable_status = matches!(
        error.status().as_u16(),
        408 | 425 | 429 | 500 | 502 | 503 | 504
    );
    if retryable_status {
        if !error.retry_hint().is_retryable() {
            return FailureClass::RemoteFailure;
        }
        return match error.status().as_u16() {
            408 => FailureClass::Timeout,
            429 => FailureClass::Overloaded,
            502..=504 => FailureClass::Unavailable,
            _ => FailureClass::RemoteServer,
        };
    }

    if error.status().is_server_error() {
        FailureClass::RemoteFailure
    } else {
        FailureClass::InvalidRequest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ErrorCode, RemoteErrorParts, RetryHint};
    use http::StatusCode;

    fn remote(
        kind: ErrorKind,
        category: ErrorCategory,
        status: StatusCode,
        retry_hint: RetryHint,
    ) -> Error {
        Error::from_remote_parts(RemoteErrorParts {
            kind,
            category,
            code: ErrorCode::framework("classified_error"),
            message: "failed".to_owned(),
            status,
            request_id: "request-1".to_owned(),
            details: None,
            retry_hint,
        })
    }

    #[test]
    fn application_status_controls_breaker_health_without_enabling_retry() {
        let rejected = remote(
            ErrorKind::Application,
            ErrorCategory::Conflict,
            StatusCode::CONFLICT,
            RetryHint::Retryable,
        );
        let failed = remote(
            ErrorKind::Application,
            ErrorCategory::Internal,
            StatusCode::INTERNAL_SERVER_ERROR,
            RetryHint::Retryable,
        );

        assert_eq!(classify_error(&rejected), FailureClass::Application);
        assert_eq!(classify_error(&failed), FailureClass::RemoteFailure);
        assert!(!classify_error(&rejected).is_retryable());
        assert!(!classify_error(&failed).is_retryable());
    }

    #[test]
    fn protocol_precedes_transient_status() {
        let malformed = remote(
            ErrorKind::Framework,
            ErrorCategory::DataLoss,
            StatusCode::BAD_GATEWAY,
            RetryHint::Retryable,
        );
        assert_eq!(classify_error(&malformed), FailureClass::Protocol);
    }

    #[test]
    fn normalized_retry_hint_controls_remote_framework_retry() {
        let retryable = remote(
            ErrorKind::Framework,
            ErrorCategory::Unavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            RetryHint::Retryable,
        );
        let terminal = remote(
            ErrorKind::Framework,
            ErrorCategory::Unavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            RetryHint::Never,
        );
        assert_eq!(classify_error(&retryable), FailureClass::Unavailable);
        assert_eq!(classify_error(&terminal), FailureClass::RemoteFailure);
    }

    #[test]
    fn remote_framework_rejections_are_not_application_failures() {
        let rejected = remote(
            ErrorKind::Framework,
            ErrorCategory::InvalidArgument,
            StatusCode::BAD_REQUEST,
            RetryHint::Never,
        );
        assert_eq!(classify_error(&rejected), FailureClass::InvalidRequest);
    }
}
