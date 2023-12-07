use crate::protocol::KrpcMsg;
use futures::Future;
use http_body::Body;
use hyper::{service::Service, Request, Response};
use std::{marker::PhantomData, thread};
use tracing::debug;

pub struct KrpcRouter<F, KF, ReqBody, Err> {
    codec_filter: F,
    filter_list: Vec<KF>,
    _req: PhantomData<ReqBody>,
    _err: PhantomData<Err>,
}

impl<F, KF, S, ReqBody, Err> KrpcRouter<F, KF, ReqBody, Err>
where
    F: Fn(Request<ReqBody>) -> S,
    S: Future,
    KF: KrpcFilter<Request = KrpcMsg, Response = KrpcMsg, Error = crate::Error>,
{
    pub fn new(codec_filter: F, filter_list: Vec<KF>) -> Self {
        return KrpcRouter {
            codec_filter,
            filter_list,
            _req: PhantomData,
            _err: PhantomData,
        };
    }
}

impl<F, KF, Ret, ReqBody, ResBody, Err> Service<Request<ReqBody>>
    for KrpcRouter<F, KF, ReqBody, Err>
where
    ReqBody: Body,
    ResBody: Body,
    F: Fn(Request<ReqBody>) -> Ret,
    Err: Into<Box<dyn std::error::Error + Send + Sync>>,
    Ret: Future<Output = Result<Response<ResBody>, Err>>,
    KF: KrpcFilter<Request = KrpcMsg, Response = KrpcMsg, Error = crate::Error> + Clone,
{
    type Response = Response<ResBody>;
    type Error = Err;
    type Future = Ret;

    fn call(&self, req: Request<ReqBody>) -> Self::Future {
        return (self.codec_filter)(req);
    }
}

#[derive(Debug, Clone, Default)]
pub struct TestFilter {}

impl KrpcFilter for TestFilter {
    type Request = KrpcMsg;

    type Response = KrpcMsg;

    type Error = crate::Error;

    type Future = crate::KrpcFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, req: Self::Response) -> Self::Future {
        let mut msg: KrpcMsg = req;
        debug!("thead_id1{:?}", thread::current().id());
        debug!("thead_id2{:?}", thread::current().id());

        msg.class_name = "test".to_string();
        debug!("thead_id3{:?}", thread::current().id());
        Box::pin(async move { Ok(msg) })
    }
}

pub trait KrpcFilter {
    type Request;

    type Response;

    /// Errors produced by the service.
    type Error;

    /// The future response value.
    type Future: Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future;
}
