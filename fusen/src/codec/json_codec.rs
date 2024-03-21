use crate::StreamBody;
use fusen_common::error::{BoxFusenError, FusenError};
use http_body::Frame;
use std::{fmt::Debug, marker::PhantomData};

use super::BodyCodec;

pub struct JsonBodyCodec<D, E> {
    _d: PhantomData<D>,
    _e: PhantomData<E>,
}

impl<D, E> JsonBodyCodec<D, E>
where
    D: bytes::Buf + Debug,
    E: std::marker::Sync + std::marker::Send,
{
    pub fn new() -> Self {
        JsonBodyCodec {
            _d: PhantomData,
            _e: PhantomData,
        }
    }
}

impl<D, E> BodyCodec<D, E> for JsonBodyCodec<D, E>
where
    D: bytes::Buf + Debug,
    E: std::marker::Sync + std::marker::Send,
{
    fn decode(&self, body: Vec<Frame<D>>) -> Result<Vec<String>, BoxFusenError> {
        let data = if body.is_empty() || body[0].is_trailers() {
            return Err(FusenError::Server("receive frame err".to_string()).boxed());
        } else {
            body[0].data_ref().unwrap().chunk()
        };
        Ok(if data.starts_with(b"[") {
            match serde_json::from_slice(&data) {
                Ok(req) => req,
                Err(err) => return Err(FusenError::Client(err.to_string()).boxed()),
            }
        } else {
            vec![String::from_utf8(data.to_vec()).unwrap()]
        })
    }

    fn encode(
        &self,
        res: Result<String, BoxFusenError>,
    ) -> Result<StreamBody<bytes::Bytes, E>, BoxFusenError> {
        let res = res?;
        let chunks = vec![Ok(Frame::data(bytes::Bytes::from(res)))];
        let stream = futures_util::stream::iter(chunks);
        let stream_body = http_body_util::StreamBody::new(stream);
        Ok(stream_body)
    }
}
