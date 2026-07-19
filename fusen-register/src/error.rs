use thiserror::Error;

#[derive(Error, Debug)]
/// Failures produced by a registry or discovery directory.
pub enum RegisterError {
    /// Provider-specific thread-safe source error.
    #[error("Error : {0}")]
    Error(Box<dyn std::error::Error + Send + Sync + 'static>),
    /// The directory no longer has any receivers.
    #[error("directory channel is closed")]
    DirectoryClosed,
    /// A service address or resource cannot be published.
    #[error("invalid service resource: {0}")]
    InvalidResource(String),
}
