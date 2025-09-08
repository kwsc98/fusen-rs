use std::collections::LinkedList;

use crate::{
    error::FusenError,
    protocol::{
        codec::body::{RequestBodyCodec, ResponseBodyCodec},
        fusen::response::HttpStatus,
    },
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
                serde_json::to_vec(&body.pop_front().unwrap())
                    .map_err(|error| FusenError::Error(Box::new(error)))?
            } else {
                serde_json::to_vec(&body).map_err(|error| FusenError::Error(Box::new(error)))?
            };
            return Ok(Bytes::from(bytes));
        }
        Ok(Bytes::new())
    }

    fn decode(
        &self,
        bytes: bytes::Bytes,
    ) -> Result<LinkedList<serde_json::Value>, crate::error::FusenError> {
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            FusenError::HttpError(HttpStatus {
                status: 400,
                message: Some(format!("{error:?}")),
            })
        })?;
        if value.is_array() {
            let valus: LinkedList<Value> = serde_json::from_value(value)
                .map_err(|error| FusenError::Error(Box::new(error)))?;
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
        Ok(Bytes::from(
            serde_json::to_vec(&value).map_err(|error| FusenError::Error(Box::new(error)))?,
        ))
    }

    fn decode(&self, bytes: bytes::Bytes) -> Result<Value, crate::error::FusenError> {
        serde_json::from_slice(&bytes).map_err(|error| FusenError::Error(Box::new(error)))
    }
}
