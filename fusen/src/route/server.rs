use bytes::Bytes;
use fusen_common::{error::FusenError, FusenContext, FusenFuture};
use hyper::{service::Service, Request, Response};
use prost::Message;
use std::sync::Arc;

use crate::{
    codec::{http_codec::FusenHttpCodec, HttpCodec},
    filter::FusenFilter,
    BoxBody, StreamBody,
};

pub struct FusenRouter<KF: 'static> {
    fusen_filter: Arc<&'static KF>,
    http_codec: Arc<FusenHttpCodec<bytes::Bytes, crate::Error>>,
}

impl<KF> FusenRouter<KF>
where
    KF: FusenFilter<Request = FusenContext, Response = FusenContext, Error = crate::Error> + Clone,
{
    pub fn new(fusen_filter: &'static KF) -> Self {
        return FusenRouter {
            fusen_filter: Arc::new(fusen_filter),
            http_codec: Arc::new(FusenHttpCodec::new()),
        };
    }
}

impl<KF> Service<Request<BoxBody<Bytes, crate::Error>>> for FusenRouter<KF>
where
    KF: FusenFilter<Request = FusenContext, Response = FusenContext, Error = crate::Error>
        + Clone
        + Send
        + 'static,
{
    type Response = Response<StreamBody<Bytes, crate::Error>>;
    type Error = FusenError;
    type Future = FusenFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, mut req: Request<BoxBody<Bytes, crate::Error>>) -> Self::Future {
        let fusen_filter = self.fusen_filter.clone();
        let http_codec = self.http_codec.clone();
        Box::pin(async move {
            let context = http_codec.decode(req).await?;
            let context = fusen_filter.call(context).await?;
            http_codec.encode(context)
        })
    }
}
