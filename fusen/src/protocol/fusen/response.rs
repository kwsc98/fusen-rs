use crate::{error::FusenError, protocol::Protocol};
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, fmt::Display};

#[derive(Debug)]
pub struct FusenResponse {
    pub protocol: Protocol,
    pub http_status: HttpStatus,
    pub headers: HashMap<String, String>,
    pub extensions: Option<HashMap<String, String>>,
    pub body: Option<Value>,
}

#[derive(Debug)]
pub struct HttpStatus {
    pub status: u16,
    pub message: Option<String>,
}

impl Default for HttpStatus {
    fn default() -> Self {
        Self {
            status: 200,
            message: None,
        }
    }
}

impl Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "(status:{}, message:{:?})", self.status, self.message)
    }
}

impl Default for FusenResponse {
    fn default() -> Self {
        Self {
            protocol: Protocol::default(),
            http_status: Default::default(),
            headers: Default::default(),
            extensions: Default::default(),
            body: Default::default(),
        }
    }
}

impl FusenResponse {
    pub fn init_response<T: Serialize>(&mut self, result: Result<T, FusenError>) {
        let mut http_status = HttpStatus::default();
        match result {
            Ok(value) => match serde_json::to_value(value) {
                Ok(value) => {
                    let _ = self.body.insert(value);
                }
                Err(error) => {
                    http_status = HttpStatus {
                        status: 500,
                        message: Some(format!("Failed to serialize response : {error:?}")),
                    };
                }
            },
            Err(error) => {
                if let FusenError::HttpError(status) = error {
                    http_status = status;
                }
            }
        };
        self.http_status = http_status;
    }
}
