use std::{error::Error as StdError, fmt, sync::Arc};

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
#[derive(Clone)]
#[non_exhaustive]
pub struct ClientError {
    kind: ClientErrorKind,
    message: Arc<str>,
    source: Option<Arc<dyn StdError + Send + Sync + 'static>>,
}

impl ClientError {
    pub(crate) fn from_message(kind: ClientErrorKind, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind,
            message: Arc::from(message.as_str()),
            source: None,
        }
    }

    pub(crate) fn with_source<E>(
        kind: ClientErrorKind,
        message: impl Into<String>,
        source: E,
    ) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        let message = message.into();
        Self {
            kind,
            message: Arc::from(message.as_str()),
            source: Some(Arc::new(source)),
        }
    }

    /// Returns the stable failure class.
    pub const fn kind(&self) -> ClientErrorKind {
        self.kind
    }

    /// Returns the public diagnostic message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the retained diagnostic source.
    pub fn source_ref(&self) -> Option<&(dyn StdError + Send + Sync + 'static)> {
        self.source.as_deref()
    }
}

impl fmt::Debug for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientError")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .field("has_source", &self.source.is_some())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "client {} failed: {}", self.kind, self.message)
    }
}

impl StdError for ClientError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
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
#[derive(Clone)]
#[non_exhaustive]
pub struct ServerError {
    kind: ServerErrorKind,
    message: Arc<str>,
    source: Option<Arc<dyn StdError + Send + Sync + 'static>>,
}

impl ServerError {
    pub(crate) fn from_message(kind: ServerErrorKind, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind,
            message: Arc::from(message.as_str()),
            source: None,
        }
    }

    pub(crate) fn with_source<E>(
        kind: ServerErrorKind,
        message: impl Into<String>,
        source: E,
    ) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        let message = message.into();
        Self {
            kind,
            message: Arc::from(message.as_str()),
            source: Some(Arc::new(source)),
        }
    }

    /// Returns the stable failure class.
    pub const fn kind(&self) -> ServerErrorKind {
        self.kind
    }

    /// Returns the public diagnostic message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the retained diagnostic source.
    pub fn source_ref(&self) -> Option<&(dyn StdError + Send + Sync + 'static)> {
        self.source.as_deref()
    }
}

impl fmt::Debug for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerError")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .field("has_source", &self.source.is_some())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "server {} failed: {}", self.kind, self.message)
    }
}

impl StdError for ServerError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_expose_sources() {
        let client = ClientError::with_source(
            ClientErrorKind::Connect,
            "connection failed",
            std::io::Error::other("client secret"),
        );
        let server = ServerError::with_source(
            ServerErrorKind::Bind,
            "bind failed",
            std::io::Error::other("server secret"),
        );

        assert!(!format!("{client:?}").contains("client secret"));
        assert!(!format!("{server:?}").contains("server secret"));
        assert!(!client.to_string().contains("client secret"));
        assert!(!server.to_string().contains("server secret"));
        assert!(client.source_ref().is_some());
        assert!(server.source_ref().is_some());
    }
}
