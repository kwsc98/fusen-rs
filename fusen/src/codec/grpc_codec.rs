use http_body::Frame;
use prost::Message;
use std::marker::PhantomData;

use crate::support::triple::get_buf;

use super::BodyCodec;

pub struct GrpcBodyCodec<D, U, T> {
    _d: PhantomData<D>,
    _u: PhantomData<U>,
    _t: PhantomData<T>,
}

impl<D, U, T> GrpcBodyCodec<D, U, T> {
    pub fn new() -> Self {
        GrpcBodyCodec {
            _d: PhantomData,
            _u: PhantomData,
            _t: PhantomData,
        }
    }
}

impl<D, U, T> BodyCodec<D> for GrpcBodyCodec<D, U, T>
where
    D: bytes::Buf,
    U: Message + Send + 'static + Default,
    T: Message + Default + Send + 'static,
{
    type DecodeType = U;

    type EncodeType = T;

    fn decode(&self, body: Frame<D>) -> Result<Self::DecodeType, crate::Error> {
        let data = body.data_ref().unwrap().chunk();
        let wrapper = Self::DecodeType::decode(&data[5..])?;
        Ok(wrapper)
    }

    fn encode(&self, res: Self::EncodeType) -> Result<bytes::Bytes, crate::Error> {
        let buf = res.encode_to_vec();
        let buf = get_buf(buf);
        Ok(bytes::Bytes::from(buf).into())
    }
}
