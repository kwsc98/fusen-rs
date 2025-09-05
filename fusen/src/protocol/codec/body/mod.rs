use std::collections::LinkedList;

use crate::error::FusenError;
use bytes::Bytes;
use serde_json::Value;

pub mod json;
pub mod triple;

pub trait RequestBodyCodec {
    fn encode(&self, bodys: LinkedList<Value>) -> Result<Bytes, FusenError>;

    fn decode(&self, bytes: Bytes) -> Result<LinkedList<Value>, FusenError>;
}

pub trait ResponseBodyCodec {
    fn encode(&self, body: Value) -> Result<Bytes, FusenError>;

    fn decode(&self, bytes: Bytes) -> Result<Value, FusenError>;
}
