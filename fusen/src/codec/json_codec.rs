use bytes::{Buf, BufMut};
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

impl<D, U, T> Default for JsonBodyCodec<D, U, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'b, D, U, T> BodyCodec<D> for JsonBodyCodec<D, U, T>
where
    D: bytes::Buf + 'b,
    U: Deserialize<'b>,
    T: Serialize,
{
    type DecodeType = U;

    type EncodeType = T;

    fn decode(&self, body: &D) -> Result<Self::DecodeType, crate::Error> {
        let data = body.chunk();
        let mut de = serde_json::Deserializer::from_reader(data.reader());
        Ok(U::deserialize(&mut de)?)
    }

    fn encode(&self, res: &Self::EncodeType) -> Result<bytes::Bytes, crate::Error> {
        let mut byte = bytes::BytesMut::new();
        let mut_bytes = &mut byte;
        serde_json::to_writer(mut_bytes.writer(), &res).map_err(Box::new)?;
        Ok(byte.into())
    }
}
