use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum Error {
    #[error("register error: {0}")]
    Register(#[source] Arc<dyn std::error::Error + Send + Sync + 'static>),
    #[error("configuration error: {0}")]
    Config(#[source] Arc<dyn std::error::Error + Send + Sync + 'static>),
    #[error("{0}")]
    Message(String),
}

impl Error {
    pub fn register<E: std::error::Error + Send + Sync + 'static>(error: E) -> Self {
        Self::Register(Arc::new(error))
    }
    pub fn config<E: std::error::Error + Send + Sync + 'static>(error: E) -> Self {
        Self::Config(Arc::new(error))
    }
}
