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
    /// A registration, selector, protocol, or endpoint is invalid.
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
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
#[error("registry {operation} failed ({kind}): {source}")]
pub struct RegistryError {
    operation: RegistryOperation,
    kind: RegistryErrorKind,
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
            source: Arc::new(source),
        }
    }

    /// Creates a classified registry error from an owned diagnostic message.
    pub fn message(
        operation: RegistryOperation,
        kind: RegistryErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self::new(operation, kind, RegistryMessage(message.into()))
    }

    /// Returns the operation that failed.
    pub const fn operation(&self) -> RegistryOperation {
        self.operation
    }

    /// Returns the stable failure classification.
    pub const fn kind(&self) -> RegistryErrorKind {
        self.kind
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
        assert!(error.to_string().contains("provider offline"));
    }
}
