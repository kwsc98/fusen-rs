use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};


#[derive(Serialize, Deserialize, Debug)]
pub enum FusenError {
    Null,
    Client(String),
    Server(String),
    Method(String),
}

unsafe impl Send for FusenError {}

unsafe impl Sync for FusenError {}

impl Display for FusenError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            FusenError::Null => write!(f, "Bad value"),
            FusenError::Client(msg) => write!(f, "FusenError::Client {}", msg),
            FusenError::Server(msg) => write!(f, "FusenError::Server {}", msg),
            FusenError::Method(msg) => write!(f, "FusenError::Method {}", msg),
        }
    }
}

impl std::error::Error for FusenError {}