use std::{error::Error, fmt, sync::Arc};

/// Stable classification for configuration validation failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConfigValidationErrorKind {
    /// One value is outside its supported range.
    OutOfRange,
    /// Two or more individually valid values are inconsistent.
    Inconsistent,
}

impl fmt::Display for ConfigValidationErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OutOfRange => "out of range",
            Self::Inconsistent => "inconsistent",
        })
    }
}

/// A safe, field-addressable configuration validation failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
#[error("invalid configuration at {field_path} ({kind}): {reason}")]
pub struct ConfigValidationError {
    kind: ConfigValidationErrorKind,
    field_path: &'static str,
    reason: &'static str,
}

impl ConfigValidationError {
    pub(crate) const fn new(
        kind: ConfigValidationErrorKind,
        field_path: &'static str,
        reason: &'static str,
    ) -> Self {
        Self {
            kind,
            field_path,
            reason,
        }
    }

    /// Returns the stable validation classification.
    pub const fn kind(&self) -> ConfigValidationErrorKind {
        self.kind
    }

    /// Returns the exact public configuration field path.
    pub const fn field_path(&self) -> &'static str {
        self.field_path
    }

    /// Returns a public, credential-free explanation.
    pub const fn reason(&self) -> &'static str {
        self.reason
    }
}

/// Stable class of a client runtime failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClientErrorKind {
    /// Runtime configuration or construction failed.
    Build,
    /// A direct endpoint or discovery connection could not be established.
    Connect,
    /// Service discovery did not become usable.
    Discovery,
    /// The client runtime is draining or closed.
    Closed,
    /// A bounded client operation exceeded its deadline.
    Timeout,
    /// Client shutdown failed before its shared deadline.
    Shutdown,
}

impl fmt::Display for ClientErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Build => "build",
            Self::Connect => "connect",
            Self::Discovery => "discovery",
            Self::Closed => "closed",
            Self::Timeout => "timeout",
            Self::Shutdown => "shutdown",
        })
    }
}

/// Failure to construct, connect, or shut down a [`ClientRuntime`](crate::ClientRuntime).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ClientError {
    kind: ClientErrorKind,
    message: String,
    source: Option<Arc<dyn Error + Send + Sync + 'static>>,
}

impl ClientError {
    pub(crate) fn message(kind: ClientErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn with_source<E>(
        kind: ClientErrorKind,
        message: impl Into<String>,
        source: E,
    ) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind,
            message: message.into(),
            source: Some(Arc::new(source)),
        }
    }

    /// Returns the stable failure class.
    pub const fn kind(&self) -> ClientErrorKind {
        self.kind
    }

    /// Returns the public diagnostic message.
    pub fn message_ref(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "client {} failed: {}", self.kind, self.message)
    }
}

impl Error for ClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

/// Stable class of a server lifecycle failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServerErrorKind {
    /// Static service, route, address, or configuration validation failed.
    Validation,
    /// The listening socket could not be bound.
    Bind,
    /// The server could not reach Ready during startup.
    Startup,
    /// The listener reached a fatal accept failure.
    Accept,
    /// One or more registry operations failed.
    Registry,
    /// The shared graceful-shutdown deadline elapsed.
    Timeout,
    /// Shutdown failed for a reason other than its deadline.
    Shutdown,
}

impl fmt::Display for ServerErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Validation => "validation",
            Self::Bind => "bind",
            Self::Startup => "startup",
            Self::Accept => "accept",
            Self::Registry => "registry",
            Self::Timeout => "timeout",
            Self::Shutdown => "shutdown",
        })
    }
}

/// Failure to validate, start, accept, register, or shut down a server.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ServerError {
    kind: ServerErrorKind,
    message: String,
    source: Option<Arc<dyn Error + Send + Sync + 'static>>,
}

impl ServerError {
    pub(crate) fn message(kind: ServerErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn with_source<E>(
        kind: ServerErrorKind,
        message: impl Into<String>,
        source: E,
    ) -> Self
    where
        E: Error + Send + Sync + 'static,
    {
        Self {
            kind,
            message: message.into(),
            source: Some(Arc::new(source)),
        }
    }

    /// Returns the stable failure class.
    pub const fn kind(&self) -> ServerErrorKind {
        self.kind
    }

    /// Returns the public diagnostic message.
    pub fn message_ref(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "server {} failed: {}", self.kind, self.message)
    }
}

impl Error for ServerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}
