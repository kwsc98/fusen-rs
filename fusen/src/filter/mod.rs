use futures::Future;
use http_body::Body;
use hyper::{service::Service, Request, Response};
use fusen_common::{FusenMsg, RpcError, RpcServer};
use std::{collections::HashMap, marker::PhantomData, sync::Arc};

pub struct FusenRouter<F, KF, ReqBody, Err> {
    codec_filter: F,
    filter_list: Arc<Vec<KF>>,
    _req: PhantomData<ReqBody>,
    _err: PhantomData<Err>,
}

impl<F, KF, S, ReqBody, Err> FusenRouter<F, KF, ReqBody, Err>
where
    F: Fn(Request<ReqBody>, Arc<Vec<KF>>) -> S,
    S: Future,
    KF: FusenFilter<Request = FusenMsg, Response = FusenMsg, Error = crate::Error>,
{
    pub fn _new(codec_filter: F, filter_list: Vec<KF>) -> Self {
        return FusenRouter {
            codec_filter,
            filter_list: Arc::new(filter_list),
            _req: PhantomData,
            _err: PhantomData,
        };
    }
}

impl<F, KF, Ret, ReqBody, ResBody, Err> Service<Request<ReqBody>>
    for FusenRouter<F, KF, ReqBody, Err>
where
    ReqBody: Body,
    ResBody: Body,
    F: Fn(Request<ReqBody>, Arc<Vec<KF>>) -> Ret,
    Err: Into<Box<dyn std::error::Error + Send + Sync>>,
    Ret: Future<Output = Result<Response<ResBody>, Err>>,
    KF: FusenFilter<Request = FusenMsg, Response = FusenMsg, Error = crate::Error> + Clone,
{
    type Response = Response<ResBody>;
    type Error = Err;
    type Future = Ret;

    fn call(&self, req: Request<ReqBody>) -> Self::Future {
        return (self.codec_filter)(req, self.filter_list.clone());
    }
}

#[derive(Clone, Default)]
pub struct RpcServerRoute {
    map: HashMap<String, Arc<Box<dyn RpcServer>>>,
}

impl RpcServerRoute {
    pub fn new(map: HashMap<String, Arc<Box<dyn RpcServer>>>) -> Self {
        return RpcServerRoute { map };
    }
}

impl FusenFilter for RpcServerRoute {
    type Request = FusenMsg;

    type Response = FusenMsg;

    type Error = crate::Error;

    type Future = crate::FusenFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let mut msg: FusenMsg = req;
        let mut class_name = msg.class_name.clone();
        if let Some(version) = &msg.version {
            class_name.push_str(":");
            class_name.push_str(version);
        }
        match self.map.get(&class_name) {
            Some(server) => {
                let server = server.clone();
                Box::pin(async move { Ok(server.invoke(msg).await) })
            }
            None => Box::pin(async move {
                msg.res = Err(RpcError::Server(format!(
                    "not find server by {}",
                    class_name
                )));
                Ok(msg)
            }),
        }
    }
}

pub trait FusenFilter {
    type Request;

    type Response;

    type Error;

    type Future: Future<Output = Result<Self::Response, Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future;
}
