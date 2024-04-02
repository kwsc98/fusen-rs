use bytes::Buf;
use fusen_common::error::FusenError;
use http_body::Frame;
use prost::Message;
use std::marker::PhantomData;

use crate::support::triple::get_buf;

use super::BodyCodec;

pub struct GrpcBodyCodec<D,E,U, T> 
{   
    _d: PhantomData<D>,
    _e: PhantomData<E>,
    _u: PhantomData<U>,
    _t: PhantomData<T>,
}

impl<D,E,U, T> GrpcBodyCodec<D,E,U, T> {
    pub fn new() -> Self {
        GrpcBodyCodec {
            _d: PhantomData,
            _e: PhantomData,
            _u: PhantomData,
            _t: PhantomData,
        }
    }
}

impl<D, E, U, T> BodyCodec<D, E> for GrpcBodyCodec<D,E,U, T>
where
    D: bytes::Buf ,
    E: std::marker::Sync + std::marker::Send,
    U: Message + Send + 'static + Default,
    T: Message + Default + Send + 'static,
{
    type DecodeType = U;

    type EncodeType = T;

    fn decode(&self, body: Vec<Frame<D>>) -> Result<Self::DecodeType, crate::Error> {
        let data = if body.is_empty() || body[0].is_trailers() {
            return Err("decode frame err".into());
        } else {
            body[0].data_ref().unwrap().chunk()
        };
        let wrapper = Self::DecodeType::decode(&data[5..])?;
        Ok(wrapper)
    }

    fn encode(&self, res: Self::EncodeType) -> Result<Frame<bytes::Bytes>, crate::Error> {
        let buf = res.encode_to_vec();
        let buf = get_buf(buf);
        Ok(Frame::data(bytes::Bytes::from(buf)))
    }
}
