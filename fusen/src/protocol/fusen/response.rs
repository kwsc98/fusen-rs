use crate::error::FusenError;
use fusen_internal_common::protocol::WireProtocol;
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, fmt::Display};

#[derive(Debug, Default)]
pub struct FusenResponse {
    pub protocol: WireProtocol,
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

impl FusenResponse {
    pub fn init_response<T: Serialize>(
        &mut self,
        result: Result<T, FusenError>,
    ) -> Result<(), FusenError> {
        let value = result?;
        self.body = Some(
            serde_json::to_value(value)
                .map_err(|error| FusenError::internal("failed to serialize response", error))?,
        );
        self.http_status = HttpStatus::default();
        Ok(())
    }
}
