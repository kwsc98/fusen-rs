use std::marker::PhantomData;

use bytes::Buf;
use http_body::Frame;

use super::BodyCodec;

pub struct JsonBodyCodec<D, E> {
    _d: PhantomData<D>,
    _e: PhantomData<E>,
}

impl<D, E> JsonBodyCodec<D, E> {
    pub fn new() -> Self {
        Self {
            _d: PhantomData,
            _e: PhantomData,
        }
    }
}

impl<D, E> BodyCodec<D, E> for JsonBodyCodec<D, E>
where
    D: bytes::Buf,
    E: std::marker::Sync + std::marker::Send,
{
    type DecodeType = Vec<String>;

    type EncodeType = Vec<String>;

    fn decode(&self, body: Vec<Frame<D>>) -> Result<Self::DecodeType, crate::Error> {
        let data = if body.is_empty() || body[0].is_trailers() {
            return Err("receive frame err".into());
        } else {
            body[0].data_ref().unwrap().chunk()
        };
        Ok(if data.starts_with(b"[") {
            match serde_json::from_slice(&data) {
                Ok(req) => req,
                Err(err) => return Err(err.into()),
            }
        } else {
            vec![String::from_utf8(data.to_vec()).unwrap()]
        })
    }

    fn encode(&self, res: Self::EncodeType) -> Result<Frame<bytes::Bytes>, crate::Error> {
        if res.is_empty() {
            return Err("encode err res is empty".into());
        }
        let res = if res.len() == 1 {
            res[0]
        } else {
            serde_json::to_string(&res)?
        };
        Ok(Frame::data(bytes::Bytes::from(res)))
    }
}
