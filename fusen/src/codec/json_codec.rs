use bytes::BufMut;
use http_body::Frame;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

use super::BodyCodec;

pub struct JsonBodyCodec<D, U, T> {
    _d: PhantomData<D>,
    _u: PhantomData<U>,
    _t: PhantomData<T>,
}

impl<D, U, T> JsonBodyCodec<D, U, T> {
    pub fn new() -> Self {
        Self {
            _d: PhantomData,
            _u: PhantomData,
            _t: PhantomData,
        }
    }
}

impl<'b, D, U, T> BodyCodec<D> for JsonBodyCodec<D, U, T>
where
    D: bytes::Buf,
    U: Deserialize<'b>,
    T: Serialize,
{
    type DecodeType = U;

    type EncodeType = T;

    fn decode(&self, body: Frame<D>) -> Result<Self::DecodeType, crate::Error> {
        let data = body.data_ref().unwrap().chunk();
        serde_json::from_slice(data).map_err(|e| e.into())
    }

    fn encode(&self, res: Self::EncodeType) -> Result<bytes::Bytes, crate::Error> {
        let byte = bytes::BytesMut::new();
        serde_json::to_writer(byte.writer(), &res).map_err(|e| Box::new(e))?;
        Ok(byte.into())
    }
}
