use std::collections::LinkedList;

use crate::{
    error::FusenError,
    protocol::codec::body::{RequestBodyCodec, ResponseBodyCodec},
};
use bytes::Bytes;
use serde_json::Value;

#[derive(Default)]
pub struct JsonCodec;

impl RequestBodyCodec for JsonCodec {
    fn encode(
        &self,
        mut body: LinkedList<serde_json::Value>,
    ) -> Result<bytes::Bytes, crate::error::FusenError> {
        if !body.is_empty() {
            let bytes = if body.len() == 1 {
                serde_json::to_vec(
                    &body.pop_front().ok_or_else(|| {
                        FusenError::InvalidRequest("request body is empty".into())
                    })?,
                )
                .map_err(|error| FusenError::internal("failed to encode JSON request", error))?
            } else {
                serde_json::to_vec(&body)
                    .map_err(|error| FusenError::internal("failed to encode JSON request", error))?
            };
            return Ok(Bytes::from(bytes));
        }
        Ok(Bytes::new())
    }

    fn decode(
        &self,
        bytes: bytes::Bytes,
    ) -> Result<LinkedList<serde_json::Value>, crate::error::FusenError> {
        if bytes.is_empty() {
            return Ok(LinkedList::new());
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| FusenError::InvalidRequest(error.to_string()))?;
        if value.is_array() {
            let valus: LinkedList<Value> = serde_json::from_value(value)
                .map_err(|error| FusenError::internal("failed to decode JSON arguments", error))?;
            Ok(valus)
        } else {
            let mut linked_list = LinkedList::new();
            linked_list.push_back(value);
            Ok(linked_list)
        }
    }
}

impl ResponseBodyCodec for JsonCodec {
    fn encode(&self, value: Value) -> Result<bytes::Bytes, crate::error::FusenError> {
        Ok(Bytes::from(serde_json::to_vec(&value).map_err(
            |error| FusenError::internal("failed to encode JSON response", error),
        )?))
    }

    fn decode(&self, bytes: bytes::Bytes) -> Result<Value, crate::error::FusenError> {
        serde_json::from_slice(&bytes)
            .map_err(|error| FusenError::InvalidRequest(error.to_string()))
    }
}
