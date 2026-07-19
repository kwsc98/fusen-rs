use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("register error: {0}")]
    Register(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
    #[error("configuration error: {0}")]
    Config(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
    #[error("{0}")]
    Message(String),
}

impl Error {
    pub fn register<E: std::error::Error + Send + Sync + 'static>(error: E) -> Self {
        Self::Register(Box::new(error))
    }
    pub fn config<E: std::error::Error + Send + Sync + 'static>(error: E) -> Self {
        Self::Config(Box::new(error))
    }
}
