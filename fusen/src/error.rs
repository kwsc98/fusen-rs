use thiserror::Error;

#[derive(Error, Debug)]
pub enum FusenError {
    #[error("Error : {0}")]
    Error(Box<dyn std::error::Error>),
    #[error("Impossible")]
    Impossible,
}
