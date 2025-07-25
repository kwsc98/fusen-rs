use std::convert::Infallible;

use bytes::Bytes;
use fusen_internal_common::BoxFuture;
use http::{Request, Response};
use http_body_util::{BodyExt, combinators::BoxBody};
use hyper::service::Service;

use crate::error::FusenError;

pub struct Router {}

impl Service<Request<hyper::body::Incoming>> for Router {
    type Response = Response<BoxBody<Bytes, Infallible>>;

    type Error = FusenError;

    type Future = BoxFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, request: Request<hyper::body::Incoming>) -> Self::Future {
        //首先进行编解码
        let request = request.map(|e| e.boxed());
        todo!()
    }
}


