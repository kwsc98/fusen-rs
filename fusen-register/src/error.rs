use thiserror::Error;

#[derive(Error, Debug)]
pub enum RegisterError {
    #[error("Error : {0}")]
    Error(Box<dyn std::error::Error>),
    #[error("Impossible")]
    Impossible,
}
