use crate::protocol::KrpcMsg;
use async_trait::async_trait;
use futures::Future;
use http_body::Body;
use hyper::{service::Service, Request, Response};
use std::marker::PhantomData;
use tracing::debug;

pub struct KrpcRouter<F, ReqBody, Err> {
    codec_filter: F,
    _req: PhantomData<ReqBody>,
    _err: PhantomData<Err>,
}

impl<F, S, ReqBody, ResBody, Err> KrpcRouter<F, ReqBody, Err>
where
    S: Future<Output = Result<Response<ResBody>, Err>>,
    F: Fn(Request<ReqBody>) -> S,
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

#[async_trait]
trait KrpcFilter<Err> {
    async fn do_run(&self, msg: KrpcMsg) -> Result<KrpcMsg, Err>;
}

pub struct KrpcRouterBuilder<ReqBody, Err, DF, EF, FL, F> {
    decoder: DF,
    encoder: EF,
    filter_list: Vec<FL>,
    _req: PhantomData<ReqBody>,
    _err: PhantomData<Err>,
    _f: PhantomData<F>,
}

impl<ReqBody, ResBody, Err, Ret, DF, EF, FL, F> KrpcRouterBuilder<ReqBody, Err, DF, EF, FL, F>
where
    ReqBody: Body,
    ResBody: Body,
    DF: Fn(Request<ReqBody>) -> KrpcMsg,
    EF: Fn(Result<KrpcMsg, Err>) -> Response<ResBody>,
    Err: Into<Box<dyn std::error::Error + Send + Sync>>,
    FL: KrpcFilter<Err>,
    Ret: Future<Output = Result<Response<ResBody>, Err>>,
    F: Fn(Request<ReqBody>) -> Ret,
{
    pub fn new(decoder: DF, encoder: EF, filter_list: Vec<FL>) -> Self {
        return KrpcRouterBuilder {
            decoder,
            encoder,
            filter_list,
            _req: PhantomData,
            _err: PhantomData,
            _f: PhantomData,
        };
    }
    pub fn build(self) -> KrpcRouter<F, ReqBody, Err> {
        let de = KrpcRouter::new(move |req: Request<ReqBody>| async move {
            let mut krpc_msg = (self.decoder)(req);
            let filter_list = self.filter_list;
            for idx in 0..filter_list.len() {
                krpc_msg = match filter_list[idx].do_run(krpc_msg).await {
                    Ok(msg) => msg,
                    Err(err) => return (self.encoder)(Err(err)),
                }
            }
            return (self.encoder)(Ok(krpc_msg));
        });
        return de;
    }
}
