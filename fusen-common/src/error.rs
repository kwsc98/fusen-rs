use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxFusenError = Box<FusenError>;

#[derive(Serialize, Deserialize, Debug)]
pub enum FusenError {
    Null,
    NotFind,
    Info(String),
}

impl FusenError {
    pub fn boxed(self) -> BoxFusenError {
        Box::new(self)
    }
}

impl From<&str> for FusenError {
    fn from(value: &str) -> Self {
        FusenError::Info(value.to_string())
    }
}

impl From<String> for FusenError {
    fn from(value: String) -> Self {
        FusenError::Info(value)
    }
}

impl From<crate::Error> for FusenError {
    fn from(value: crate::Error) -> Self {
        let msg = value.to_string();
        match msg.as_str() {
            "404" => FusenError::NotFind,
            _ => FusenError::Info(msg),
        }
    }
}
impl From<http::Error> for FusenError {
    fn from(value: http::Error) -> Self {
        FusenError::Info(value.to_string())
    }
}

unsafe impl Send for FusenError {}

unsafe impl Sync for FusenError {}

impl Display for FusenError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            FusenError::Null => write!(f, "null value"),
            FusenError::Info(msg) => write!(f, "{}", msg),
            FusenError::NotFind => write!(f, "404",),
        }
    }
}

impl std::error::Error for FusenError {}
