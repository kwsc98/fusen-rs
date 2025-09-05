use std::collections::LinkedList;

use bytes::Bytes;
use prost::Message;
use serde_json::Value;

use crate::{
    error::FusenError,
    protocol::codec::body::{
        RequestBodyCodec, ResponseBodyCodec,
        triple::support::{TripleRequestWrapper, TripleResponseWrapper},
    },
};

pub mod support;

#[derive(Default)]
pub struct TripleCodec;

impl RequestBodyCodec for TripleCodec {
    fn encode(
        &self,
        bodys: LinkedList<serde_json::Value>,
    ) -> Result<bytes::Bytes, crate::error::FusenError> {
        let mut values = vec![];
        for body in bodys {
            values.push(
                serde_json::to_vec(&body).map_err(|error| FusenError::Error(Box::new(error)))?,
            );
        }
        let triple_request_wrapper = TripleRequestWrapper::encode(values);
        let bytes = triple_request_wrapper.encode_to_vec();
        Ok(Bytes::from(get_buf(bytes)))
    }

    fn decode(
        &self,
        bytes: bytes::Bytes,
    ) -> Result<LinkedList<serde_json::Value>, crate::error::FusenError> {
        let triple_request_wrapper = <TripleRequestWrapper as Message>::decode(&bytes[5..])
            .map_err(|error| FusenError::Error(Box::new(error)))?;
        let values = triple_request_wrapper.decode();
        let mut body = LinkedList::new();
        for value in values {
            body.push_back(
                serde_json::from_slice(&value)
                    .map_err(|error| FusenError::Error(Box::new(error)))?,
            );
        }
        Ok(body)
    }
}

impl ResponseBodyCodec for TripleCodec {
    fn encode(&self, body: Value) -> Result<bytes::Bytes, crate::error::FusenError> {
        let values =
            serde_json::to_vec(&body).map_err(|error| FusenError::Error(Box::new(error)))?;
        let triple_request_wrapper = TripleResponseWrapper::encode(values);
        let bytes = triple_request_wrapper.encode_to_vec();
        Ok(Bytes::from(get_buf(bytes)))
    }

    fn decode(&self, bytes: bytes::Bytes) -> Result<Value, crate::error::FusenError> {
        let triple_response_wrapper = <TripleResponseWrapper as Message>::decode(&bytes[5..])
            .map_err(|error| FusenError::Error(Box::new(error)))?;
        let value = triple_response_wrapper.decode();
        let body =
            serde_json::from_slice(&value).map_err(|error| FusenError::Error(Box::new(error)))?;
        Ok(body)
    }
}

fn get_buf(mut data: Vec<u8>) -> Vec<u8> {
    let mut len = data.len();
    let mut u8_array = vec![0_u8; 5];
    for idx in (1..5).rev() {
        u8_array[idx] = len as u8;
        len >>= 8;
    }
    u8_array.append(&mut data);
    u8_array
}
