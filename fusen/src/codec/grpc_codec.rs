use crate::{
    support::triple::{TripleExceptionWrapper, TripleRequestWrapper, TripleResponseWrapper},
    StreamBody,
};
use fusen_common::error::FusenError;
use http::{HeaderMap, HeaderValue};
use http_body::Frame;
use prost::Message;
use std::{fmt::Debug, marker::PhantomData};

use super::BodyCodec;

pub struct GrpcBodyCodec<D, E> {
    _d: PhantomData<D>,
    _e: PhantomData<E>,
}

impl<D, E> GrpcBodyCodec<D, E>
where
    D: bytes::Buf + Debug,
    E: std::marker::Sync + std::marker::Send,
{
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
    E: std::marker::Sync + std::marker::Send,
{
    fn decode(&self, body: Vec<Frame<D>>) -> Result<Vec<String>, FusenError> {
        let data = if body.is_empty() || body[0].is_trailers() {
            return Err(FusenError::Server("receive frame err".to_string()));
        } else {
            body[0].data_ref().unwrap().chunk()
        };
        let triple_request_wrapper = TripleRequestWrapper::decode(&data[5..])
            .map_err(|e| FusenError::Server(e.to_string()))?;
        Ok(triple_request_wrapper.get_req())
    }

    fn encode(
        &self,
        res: Result<String, FusenError>,
    ) -> Result<StreamBody<bytes::Bytes, E>, FusenError> {
        let mut status = "0";
        let mut message = String::from("success");
        let mut trailers = HeaderMap::new();
        let res_data = match res {
            Ok(data) => bytes::Bytes::from(TripleResponseWrapper::get_buf(data)),
            Err(err) => bytes::Bytes::from(TripleExceptionWrapper::get_buf({
                message = match err {
                    FusenError::Client(msg) => {
                        status = "90";
                        msg
                    }
                    FusenError::Method(msg) => {
                        status = "91";
                        msg
                    }
                    FusenError::Null => {
                        status = "92";
                        "FusenError::Null".to_string()
                    }
                    FusenError::ResourceEmpty(msg) => {
                        status = "93";
                        msg
                    }
                    FusenError::Server(msg) => {
                        status = "94";
                        msg
                    }
                };
                message.clone()
            })),
        };
        trailers.insert("grpc-status", HeaderValue::from_str(status).unwrap());
        trailers.insert("grpc-message", HeaderValue::from_str(&message).unwrap());

        let chunks = vec![Ok(Frame::data(res_data)), Ok(Frame::trailers(trailers))];
        let stream = futures_util::stream::iter(chunks);
        let stream_body = http_body_util::StreamBody::new(stream);
        Ok(stream_body)
    }
}
