use crate::{BoxBody, StreamBody};
use fusen_common::error::FusenError;
use http_body::Frame;
use http_body_util::BodyExt;
use std::{error::Error, fmt::Debug, marker::PhantomData};

use super::BodyCodec;

pub struct GrpcBodyCodec<D, E> {
    _d: PhantomData<D>,
    _e: PhantomData<E>,
}

impl<D, E> GrpcBodyCodec<D, E> {
    pub fn new() -> Self {
        GrpcBodyCodec {
            _d: PhantomData,
            _e: PhantomData,
        }
    }
}

impl<D, E> BodyCodec<D, E> for GrpcBodyCodec<D, E>
where
    D: bytes::Buf + Debug,
    E: Error,
{
    async fn decode(&self, mut body: BoxBody<D, E>) -> Result<Vec<String>, FusenError> {
        let mut vec: Vec<D> = vec![];
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(frame) => {
                    if frame.is_data() {
                        vec.push(frame.into_data().unwrap());
                    } else {
                        break;
                    }
                }
                Err(err) => return Err(FusenError::Client(err.to_string())),
            }
        }
        let data = if vec.is_empty() {
            return Err(FusenError::Client("err req".to_string()));
        } else {
            vec[0].chunk()
        };
        Ok(if data.starts_with(b"[") {
            match serde_json::from_slice(&data) {
                Ok(req) => req,
                Err(err) => return Err(FusenError::Client(err.to_string())),
            }
        } else {
            vec![String::from_utf8(data.to_vec()).unwrap()]
        })
    }

    async fn encode(
        &self,
        res: Result<String, FusenError>,
    ) -> Result<StreamBody<bytes::Bytes, E>, FusenError> {
        let res = res?;
        let chunks = vec![Ok(Frame::data(bytes::Bytes::from(res)))];
        let stream = futures_util::stream::iter(chunks);
        let stream_body = http_body_util::StreamBody::new(stream);
        Ok(stream_body)
    }
}
