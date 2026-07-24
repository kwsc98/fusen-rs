use crate::error::FusenError;
use fusen_contract::WireProtocol;
use http::{HeaderMap, StatusCode};
use serde::Serialize;
use serde_json::Value;

/// Complete typed RPC response returned by middleware and framework terminals.
#[derive(Debug)]
pub struct RpcResponse {
    /// HTTP status associated with the RPC response.
    pub status: StatusCode,
    /// Response headers safe for middleware to inspect or modify.
    pub headers: HeaderMap,
    /// JSON response value, when the method returned a body.
    pub body: Option<Value>,
    pub(crate) protocol: WireProtocol,
}

impl Default for RpcResponse {
    fn default() -> Self {
        Self {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: None,
            protocol: WireProtocol::default(),
        }
    }
}

impl RpcResponse {
    /// Creates a response with the supplied HTTP status and no body.
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            ..Self::default()
        }
    }

    pub(crate) fn init_response<T: Serialize>(
        &mut self,
        result: Result<T, FusenError>,
    ) -> Result<(), FusenError> {
        let value = result?;
        self.body = Some(
            serde_json::to_value(value)
                .map_err(|error| FusenError::internal("failed to serialize response", error))?,
        );
        self.status = StatusCode::OK;
        Ok(())
    }

    /// Framework entry used by generated service dispatch.
    #[doc(hidden)]
    pub fn __from_result<T: Serialize>(
        result: Result<T, FusenError>,
        protocol: WireProtocol,
    ) -> Result<Self, FusenError> {
        let mut response = Self {
            protocol,
            ..Self::default()
        };
        response.init_response(result)?;
        Ok(response)
    }
}
