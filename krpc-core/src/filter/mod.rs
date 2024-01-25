use futures::Future;
use http_body::Body;
use hyper::{service::Service, Request, Response};
use krpc_common::{KrpcMsg, RpcError, RpcServer};
use std::{collections::HashMap, marker::PhantomData, sync::Arc};

pub struct KrpcRouter<F, KF, ReqBody, Err> {
    codec_filter: F,
    filter_list: Arc<Vec<KF>>,
    _req: PhantomData<ReqBody>,
    _err: PhantomData<Err>,
}

impl<F, KF, S, ReqBody, Err> KrpcRouter<F, KF, ReqBody, Err>
where
    F: Fn(Request<ReqBody>, Arc<Vec<KF>>) -> S,
    S: Future,
    KF: KrpcFilter<Request = KrpcMsg, Response = KrpcMsg, Error = crate::Error>,
{
    pub fn new(codec_filter: F, filter_list: Vec<KF>) -> Self {
        return KrpcRouter {
            codec_filter,
            filter_list: Arc::new(filter_list),
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
    F: Fn(Request<ReqBody>, Arc<Vec<KF>>) -> Ret,
    Err: Into<Box<dyn std::error::Error + Send + Sync>>,
    Ret: Future<Output = Result<Response<ResBody>, Err>>,
    KF: KrpcFilter<Request = KrpcMsg, Response = KrpcMsg, Error = crate::Error> + Clone,
{
    type Response = Response<ResBody>;
    type Error = Err;
    type Future = Ret;

    fn call(&self, req: Request<ReqBody>) -> Self::Future {
        return (self.codec_filter)(req, self.filter_list.clone());
    }
}

#[derive(Clone, Default)]
pub struct Filter {
    map: HashMap<String, Arc<Box<dyn RpcServer>>>,
}

impl Filter {
    pub fn new(map: HashMap<String, Arc<Box<dyn RpcServer>>>) -> Self {
        return Filter { map };
    }
}

impl KrpcFilter for Filter {
    type Request = KrpcMsg;

    type Response = KrpcMsg;

    type Error = crate::Error;

    type Future = crate::KrpcFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, req: Self::Response) -> Self::Future {
        let mut msg: KrpcMsg = req;
        let class_name = (msg.class_name.clone() + ":" + &msg.version).clone();
        match self.map.get(&class_name) {
            Some(server) => {
                let server = server.clone();
                Box::pin(async move { Ok(server.invoke(msg).await) })
            },
            None => Box::pin(async move {
                msg.res = Err(RpcError::Server(format!("not find server by {}",class_name))); 
                Ok(msg)
            })
        }
    }
}


pub trait KrpcFilter {
    type Request;

    type Response;

    type Error;

    type Future: Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future;
}
