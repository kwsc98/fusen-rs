use std::{fmt, sync::Arc};
use thiserror::Error;

/// Registry operation that produced an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RegistryOperation {
    /// A provider rejected a registration before activation.
    PrepareRegistration,
    /// Publishing a prepared registration failed.
    ActivateRegistration,
    /// Removing a prepared registration failed.
    CloseRegistration,
    /// A provider rejected a subscription before activation.
    PrepareSubscription,
    /// Starting a prepared subscription failed.
    ActivateSubscription,
    /// Stopping a prepared subscription failed.
    CloseSubscription,
    /// Reading or publishing a directory snapshot failed.
    Directory,
}

impl fmt::Display for RegistryOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PrepareRegistration => "prepare registration",
            Self::ActivateRegistration => "activate registration",
            Self::CloseRegistration => "close registration",
            Self::PrepareSubscription => "prepare subscription",
            Self::ActivateSubscription => "activate subscription",
            Self::CloseSubscription => "close subscription",
            Self::Directory => "access directory",
        })
    }
}

/// Stable classification for registry failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RegistryErrorKind {
    /// The provider or its backing service is unavailable.
    Unavailable,
    /// A configured operation deadline elapsed.
    Timeout,
    /// The provider rejected the supplied credentials or identity.
    Unauthorized,
    /// A registration, selector, capability declaration, or endpoint is invalid.
    InvalidResource,
    /// The requested resource conflicts with existing provider state.
    Conflict,
    /// The operation was closed before activation completed.
    Cancelled,
    /// Provider cleanup ended without publishing a terminal result.
    CleanupAborted,
    /// An invariant, task, or provider implementation failed internally.
    Internal,
}

impl RegistryErrorKind {
    /// Returns whether a fresh provider operation may succeed without changing its input.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Unavailable | Self::Timeout)
    }
}

impl fmt::Display for RegistryErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Unauthorized => "unauthorized",
            Self::InvalidResource => "invalid resource",
            Self::Conflict => "conflict",
            Self::Cancelled => "cancelled",
            Self::CleanupAborted => "cleanup aborted",
            Self::Internal => "internal",
        })
    }
}

/// Cloneable registry failure with stable operation and kind metadata.
#[derive(Error, Clone)]
#[non_exhaustive]
#[error("registry {operation} failed ({kind}): {message}")]
pub struct RegistryError {
    operation: RegistryOperation,
    kind: RegistryErrorKind,
    message: Arc<str>,
    #[source]
    source: Arc<dyn std::error::Error + Send + Sync + 'static>,
}

impl RegistryError {
    /// Creates a classified registry error while retaining its provider source.
    pub fn new<E>(operation: RegistryOperation, kind: RegistryErrorKind, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self {
            operation,
            kind,
            message: Arc::from(default_message(kind)),
            source: Arc::new(source),
        }
    }

    /// Creates a classified registry error from a safe public diagnostic message.
    ///
    /// The message is included in [`Debug`](fmt::Debug) and [`Display`](fmt::Display) output.
    pub fn message(
        operation: RegistryOperation,
        kind: RegistryErrorKind,
        message: impl Into<String>,
    ) -> Self {
        let message = message.into();
        Self {
            operation,
            kind,
            message: Arc::from(message.as_str()),
            source: Arc::new(RegistryMessage(message)),
        }
    }

    /// Returns the operation that failed.
    pub const fn operation(&self) -> RegistryOperation {
        self.operation
    }

    /// Returns the stable failure classification.
    pub const fn kind(&self) -> RegistryErrorKind {
        self.kind
    }

    /// Returns the safe public diagnostic message.
    pub fn safe_message(&self) -> &str {
        &self.message
    }

    /// Returns whether a fresh provider operation may succeed without changing its input.
    pub const fn is_retryable(&self) -> bool {
        self.kind.is_retryable()
    }

    /// Returns the retained provider or framework source.
    pub fn source_ref(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self.source.as_ref()
    }
}

impl fmt::Debug for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistryError")
            .field("operation", &self.operation)
            .field("kind", &self.kind)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

const fn default_message(kind: RegistryErrorKind) -> &'static str {
    match kind {
        RegistryErrorKind::Unavailable => "registry provider is unavailable",
        RegistryErrorKind::Timeout => "registry operation timed out",
        RegistryErrorKind::Unauthorized => "registry provider rejected the operation",
        RegistryErrorKind::InvalidResource => "registry resource is invalid",
        RegistryErrorKind::Conflict => "registry resource conflicts with existing state",
        RegistryErrorKind::Cancelled => "registry operation was cancelled",
        RegistryErrorKind::CleanupAborted => "registry cleanup did not complete",
        RegistryErrorKind::Internal => "registry operation failed internally",
    }
}

#[derive(Debug)]
struct RegistryMessage(String);

impl fmt::Display for RegistryMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RegistryMessage {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_preserve_operation_kind_and_source() {
        let error = RegistryError::new(
            RegistryOperation::ActivateSubscription,
            RegistryErrorKind::Unavailable,
            std::io::Error::other("provider offline"),
        );

        assert_eq!(error.operation(), RegistryOperation::ActivateSubscription);
        assert_eq!(error.kind(), RegistryErrorKind::Unavailable);
        assert!(error.is_retryable());
        assert_eq!(error.safe_message(), "registry provider is unavailable");
        assert!(error.source_ref().to_string().contains("provider offline"));
        assert!(
            std::error::Error::source(&error)
                .unwrap()
                .to_string()
                .contains("provider offline")
        );
    }

    #[test]
    fn formatting_shows_safe_message_without_exposing_provider_source() {
        let error = RegistryError::new(
            RegistryOperation::ActivateSubscription,
            RegistryErrorKind::Unavailable,
            std::io::Error::other("provider-token=secret"),
        );

        let debug = format!("{error:?}");
        let display = error.to_string();
        assert!(debug.contains("ActivateSubscription"));
        assert!(debug.contains("Unavailable"));
        assert!(debug.contains("registry provider is unavailable"));
        assert!(display.contains("registry provider is unavailable"));
        assert!(!debug.contains("provider-token=secret"));
        assert!(!display.contains("provider-token=secret"));
    }

    #[test]
    fn explicit_public_message_is_preserved() {
        let error = RegistryError::message(
            RegistryOperation::Directory,
            RegistryErrorKind::InvalidResource,
            "directory snapshot is invalid",
        );

        assert_eq!(error.safe_message(), "directory snapshot is invalid");
        assert!(format!("{error:?}").contains("directory snapshot is invalid"));
        assert!(error.to_string().contains("directory snapshot is invalid"));
        assert_eq!(
            error.source_ref().to_string(),
            "directory snapshot is invalid"
        );
    }
}
