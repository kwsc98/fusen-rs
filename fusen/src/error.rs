use crate::protocol::fusen::response::HttpStatus;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FusenError {
    #[error("Error : {0}")]
    Error(Box<dyn std::error::Error>),

    #[error("HttpStatus : {0}")]
    HttpError(HttpStatus),

    #[error("Impossible")]
    Impossible,
}

unsafe impl Send for FusenError {}

unsafe impl Sync for FusenError {}