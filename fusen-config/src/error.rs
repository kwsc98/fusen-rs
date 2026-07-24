use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
/// Cloneable configuration and provider integration failure.
pub enum Error {
    /// Registration provider initialization failed.
    #[error("register error: {0}")]
    Register(#[source] Arc<dyn std::error::Error + Send + Sync + 'static>),
    /// Configuration parsing or provider access failed.
    #[error("configuration error: {0}")]
    Config(#[source] Arc<dyn std::error::Error + Send + Sync + 'static>),
    /// A validated configuration operation could not be completed.
    #[error("{0}")]
    Message(String),
}

impl Error {
    /// Wraps a registration provider error.
    pub fn register<E: std::error::Error + Send + Sync + 'static>(error: E) -> Self {
        Self::Register(Arc::new(error))
    }
    /// Wraps a configuration source or parser error.
    pub fn config<E: std::error::Error + Send + Sync + 'static>(error: E) -> Self {
        Self::Config(Arc::new(error))
    }
}
