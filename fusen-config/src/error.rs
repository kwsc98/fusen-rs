use std::{fmt, sync::Arc};
use thiserror::Error;

/// Configuration operation that produced an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConfigOperation {
    /// Reading a static configuration file failed.
    Read,
    /// Deserializing configuration content failed.
    Parse,
    /// A source rejected a key before activation.
    Prepare,
    /// Fetching and subscribing to a prepared source failed.
    Activate,
    /// Publishing a provider update failed.
    Publish,
    /// Removing a provider listener failed.
    Close,
    /// Waiting for a typed update failed.
    Watch,
}

impl fmt::Display for ConfigOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Read => "read",
            Self::Parse => "parse",
            Self::Prepare => "prepare",
            Self::Activate => "activate",
            Self::Publish => "publish",
            Self::Close => "close",
            Self::Watch => "watch",
        })
    }
}

/// Stable classification for configuration failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConfigErrorKind {
    /// A key, format, or source setting is invalid.
    InvalidInput,
    /// The requested serialization format is unsupported.
    UnsupportedFormat,
    /// Static input could not be read.
    Io,
    /// Configuration content could not be deserialized.
    InvalidData,
    /// The provider or its backing service is unavailable.
    Unavailable,
    /// The provider rejected the configured credentials.
    Unauthorized,
    /// A configured operation deadline elapsed.
    Timeout,
    /// The handle was closed before activation completed.
    Cancelled,
    /// Provider cleanup ended without a terminal result.
    CleanupAborted,
    /// An invariant, task, or provider failed internally.
    Internal,
}

impl ConfigErrorKind {
    /// Returns whether a fresh provider operation may succeed without changing its input.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Unavailable | Self::Timeout)
    }
}

impl fmt::Display for ConfigErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput => "invalid input",
            Self::UnsupportedFormat => "unsupported format",
            Self::Io => "I/O",
            Self::InvalidData => "invalid data",
            Self::Unavailable => "unavailable",
            Self::Unauthorized => "unauthorized",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::CleanupAborted => "cleanup aborted",
            Self::Internal => "internal",
        })
    }
}

/// Cloneable configuration failure with stable operation and kind metadata.
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
#[error("configuration {operation} failed ({kind}): {source}")]
pub struct ConfigError {
    operation: ConfigOperation,
    kind: ConfigErrorKind,
    #[source]
    source: Arc<dyn std::error::Error + Send + Sync + 'static>,
}

impl ConfigError {
    /// Creates a classified error while retaining its provider or parser source.
    pub fn new<E>(operation: ConfigOperation, kind: ConfigErrorKind, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self {
            operation,
            kind,
            source: Arc::new(source),
        }
    }

    /// Creates a classified error from an owned diagnostic message.
    pub fn message(
        operation: ConfigOperation,
        kind: ConfigErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self::new(operation, kind, ConfigMessage(message.into()))
    }

    /// Returns the operation that failed.
    pub const fn operation(&self) -> ConfigOperation {
        self.operation
    }

    /// Returns the stable failure classification.
    pub const fn kind(&self) -> ConfigErrorKind {
        self.kind
    }

    /// Returns whether a fresh provider operation may succeed without changing its input.
    pub const fn is_retryable(&self) -> bool {
        self.kind.is_retryable()
    }

    /// Returns the retained provider or parser source.
    pub fn source_ref(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self.source.as_ref()
    }
}

#[derive(Debug)]
struct ConfigMessage(String);

impl fmt::Display for ConfigMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ConfigMessage {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_metadata_and_source_are_preserved() {
        let error = ConfigError::new(
            ConfigOperation::Activate,
            ConfigErrorKind::Unavailable,
            std::io::Error::other("provider offline"),
        );

        assert_eq!(error.operation(), ConfigOperation::Activate);
        assert_eq!(error.kind(), ConfigErrorKind::Unavailable);
        assert!(error.is_retryable());
        assert!(error.to_string().contains("provider offline"));
    }
}
