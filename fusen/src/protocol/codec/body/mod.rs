use crate::error::FusenError;
use bytes::Bytes;
use serde_json::Value;

pub mod json;
pub mod triple;

pub trait RequestBodyCodec {
    fn encode(&self, bodys: Vec<Value>) -> Result<Bytes, FusenError>;

    fn decode(&self, bytes: Bytes) -> Result<Vec<Value>, FusenError>;
}

pub trait ResponseBodyCodec {
    fn encode(&self, body: Value) -> Result<Bytes, FusenError>;

    fn decode(&self, bytes: Bytes) -> Result<Value, FusenError>;
}
