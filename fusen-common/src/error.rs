use std::fmt::{self, Display, Formatter};

use serde::{Deserialize, Serialize};

pub type BoxFusenError = Box<FusenError>;

#[derive(Serialize, Deserialize, Debug)]
pub enum FusenError {
    Null,
    ResourceEmpty(String),
    Client(String),
    Server(String),
    Method(String),
}

impl FusenError {
    pub fn boxed(self) -> BoxFusenError {
        Box::new(self)
    }
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
            FusenError::ResourceEmpty(msg) => write!(f, "FusenError::ResourceEmpty {}", msg),
        }
    }
}

impl std::error::Error for FusenError {}
