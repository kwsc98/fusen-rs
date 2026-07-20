use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
/// Failures produced by a registry or discovery directory.
pub enum RegisterError {
    /// Provider-specific thread-safe source error.
    #[error("Error : {0}")]
    Error(#[source] Arc<dyn std::error::Error + Send + Sync + 'static>),
    /// The directory no longer has any receivers.
    #[error("directory channel is closed")]
    DirectoryClosed,
    /// A service address or resource cannot be published.
    #[error("invalid service resource: {0}")]
    InvalidResource(String),
}

impl RegisterError {
    /// Wraps a provider-specific error in cloneable shared storage.
    pub fn provider<E>(error: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Error(Arc::new(error))
    }
}
