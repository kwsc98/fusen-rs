use crate::{
    error::FusenError,
    protocol::codec::body::{RequestBodyCodec, ResponseBodyCodec},
};
use bytes::Bytes;
use serde_json::Value;

#[derive(Default)]
pub struct JsonCodec;

impl RequestBodyCodec for JsonCodec {
    fn encode(&self, body: Value) -> Result<Bytes, FusenError> {
        serde_json::to_vec(&body)
            .map(Bytes::from)
            .map_err(|error| FusenError::internal("failed to encode JSON request", error))
    }

    fn decode(&self, bytes: Bytes) -> Result<Value, FusenError> {
        serde_json::from_slice(&bytes)
            .map_err(|error| FusenError::InvalidRequest(error.to_string()))
    }
}

impl ResponseBodyCodec for JsonCodec {
    fn encode(&self, value: Value) -> Result<Bytes, FusenError> {
        serde_json::to_vec(&value)
            .map(Bytes::from)
            .map_err(|error| FusenError::internal("failed to encode JSON response", error))
    }

    fn decode(&self, bytes: Bytes) -> Result<Value, FusenError> {
        serde_json::from_slice(&bytes)
            .map_err(|error| FusenError::InvalidResponse(error.to_string()))
    }
}
