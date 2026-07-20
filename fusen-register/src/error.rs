use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
#[non_exhaustive]
/// Failures produced by a registry or discovery directory.
pub enum RegisterError {
    /// Provider-specific thread-safe source error.
    #[error("registry provider error: {0}")]
    Provider(#[source] Arc<dyn std::error::Error + Send + Sync + 'static>),
    /// Every provider writer for a directory has been dropped.
    #[error("service directory is closed")]
    DirectoryClosed,
    /// A service address or resource cannot be published.
    #[error("invalid service resource: {0}")]
    InvalidResource(String),
    /// Subscription cleanup ended without publishing a result.
    #[error("subscription cleanup was aborted")]
    CleanupAborted,
}

impl RegisterError {
    /// Wraps a provider-specific error in cloneable shared storage.
    pub fn provider<E>(error: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Provider(Arc::new(error))
    }
}
