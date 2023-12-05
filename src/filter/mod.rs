use crate::protocol::KrpcMsg;
use futures::Future;
use http_body::Body;
use hyper::{service::Service, Request, Response};
use std::marker::PhantomData;

pub struct KrpcRouter<F, ReqBody, Err> {
    codec_filter: F,
    _req: PhantomData<ReqBody>,
    _err: PhantomData<Err>,
}

impl<F, S, ReqBody, Err> KrpcRouter<F, ReqBody, Err>
where
    F: Fn(Request<ReqBody>) -> S,
    S: Future,
{
    pub fn new(codec_filter: F) -> Self {
        return KrpcRouter {
            codec_filter,
            _req: PhantomData,
            _err: PhantomData,
        };
    }
}

impl<F, Ret, ReqBody, ResBody, Err> Service<Request<ReqBody>> for KrpcRouter<F, ReqBody, Err>
where
    ReqBody: Body,
    ResBody: Body,
    F: Fn(Request<ReqBody>) -> Ret,
    Err: Into<Box<dyn std::error::Error + Send + Sync>>,
    Ret: Future<Output = Result<Response<ResBody>, Err>>,
{
    type Response = Response<ResBody>;
    type Error = Err;
    type Future = Ret;

    fn call(&self, req: Request<ReqBody>) -> Self::Future {
        return (self.codec_filter)(req);
    }
}

pub trait KrpcFilter<Request> {
    type Response;

    type Error;

    type Future: Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&mut self, req: Request) -> Self::Future;
}
