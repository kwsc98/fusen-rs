use http_body::Frame;
use std::marker::PhantomData;

use super::BodyCodec;

pub struct JsonBodyCodec<D> {
    _d: PhantomData<D>,
}

impl<D> JsonBodyCodec<D> {
    pub fn new() -> Self {
        Self { _d: PhantomData }
    }
}

impl<D> BodyCodec<D> for JsonBodyCodec<D>
where
    D: bytes::Buf,
{
    type DecodeType = Vec<String>;

    type EncodeType = Vec<String>;

    fn decode(&self, body: Vec<Frame<D>>) -> Result<Vec<String>, crate::Error> {
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

    fn encode(&self, mut res: Vec<String>) -> Result<Frame<bytes::Bytes>, crate::Error> {
        if res.is_empty() {
            return Err("encode err res is empty".into());
        }
        let res = if res.len() == 1 {
            res.remove(0)
        } else {
            serde_json::to_string(&res)?
        };
        Ok(Frame::data(bytes::Bytes::from(res)))
    }
}
