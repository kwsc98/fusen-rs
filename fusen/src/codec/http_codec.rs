use std::{collections::HashMap, convert::Infallible};

use super::{
    request_codec::RequestHandler,
    response_codec::{ResponseCodec, ResponseHandler},
    HttpCodec,
};
use crate::{codec::request_codec::RequestCodec, BoxBody};
use bytes::Bytes;
use fusen_common::FusenContext;
use http_body_util::BodyExt;

pub struct FusenHttpCodec {
    request_handle: RequestHandler,
    response_handle: ResponseHandler,
}

impl FusenHttpCodec {
    pub fn new(path_cahce: HashMap<String, (String, String)>) -> Self {
        FusenHttpCodec {
            request_handle: RequestHandler::new(path_cahce),
            response_handle: ResponseHandler::new(),
        }
    }
}

impl HttpCodec<Bytes, hyper::Error> for FusenHttpCodec {
    async fn decode(
        &self,
        request: http::Request<BoxBody<Bytes, hyper::Error>>,
    ) -> Result<FusenContext, crate::Error> {
        self.request_handle.decode(request.map(|e| e.boxed())).await
    }

    async fn encode(
        &self,
        context: fusen_common::FusenContext,
    ) -> Result<http::Response<BoxBody<bytes::Bytes, Infallible>>, crate::Error> {
        self.response_handle.encode(&context)
    }
}
