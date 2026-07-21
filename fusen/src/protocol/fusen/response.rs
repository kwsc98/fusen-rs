use crate::error::FusenError;
use fusen_contract::WireProtocol;
use http::{HeaderMap, StatusCode};
use serde::Serialize;
use serde_json::Value;
use std::fmt::Display;

#[derive(Debug, Default)]
pub struct FusenResponse {
    pub protocol: WireProtocol,
    pub http_status: HttpStatus,
    pub headers: HeaderMap,
    pub body: Option<Value>,
}

#[derive(Debug)]
pub struct HttpStatus {
    pub status: StatusCode,
    pub message: Option<String>,
}

impl Default for HttpStatus {
    fn default() -> Self {
        Self {
            status: StatusCode::OK,
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
