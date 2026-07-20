use crate::error::FusenError;
use bytes::Bytes;
use serde_json::Value;

pub mod json;

pub trait RequestBodyCodec {
    fn encode(&self, body: Value) -> Result<Bytes, FusenError>;

    fn decode(&self, bytes: Bytes) -> Result<Value, FusenError>;
}

pub trait ResponseBodyCodec {
    fn encode(&self, body: Value) -> Result<Bytes, FusenError>;

    fn decode(&self, bytes: Bytes) -> Result<Value, FusenError>;
}
