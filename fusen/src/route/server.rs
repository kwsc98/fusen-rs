use bytes::Bytes;
use fusen_common::{
    error::{BoxFusenError, FusenError},
    FusenContext, FusenFuture,
};
use http_body_util::BodyExt;
use hyper::{service::Service, Request, Response};
use std::sync::Arc;

use crate::{
    codec::{http_codec::FusenHttpCodec, HttpCodec},
    filter::FusenFilter,
    get_empty_body, StreamBody,
};

#[derive(Clone)]
pub struct FusenRouter<KF: 'static> {
    fusen_filter: Arc<&'static KF>,
    http_codec: Arc<FusenHttpCodec>,
}

impl<KF> FusenRouter<KF>
where
    KF: FusenFilter<Request = FusenContext, Response = FusenContext, Error = FusenError> + Clone,
{
    pub fn new(fusen_filter: &'static KF) -> Self {
        return FusenRouter {
            fusen_filter: Arc::new(fusen_filter),
            http_codec: Arc::new(FusenHttpCodec::new()),
        };
    }

    async fn call(
        req: Request<hyper::body::Incoming>,
        http_codec: Arc<FusenHttpCodec>,
        fusen_filter: Arc<&'static KF>,
    ) -> Result<Response<StreamBody<Bytes, hyper::Error>>, FusenError> {
        let req = req.map(|e| e.boxed());
        let context = http_codec.as_ref().decode(req).await?;
        let context = fusen_filter.call(context).await?;
        let res = http_codec.encode(context).await?;
        Ok(res)
    }
}

impl<KF> Service<Request<hyper::body::Incoming>> for FusenRouter<KF>
where
    KF: FusenFilter<Request = FusenContext, Response = FusenContext, Error = FusenError>
        + Clone
        + Send
        + 'static
        + Sync,
{
    type Response = Response<StreamBody<Bytes, hyper::Error>>;
    type Error = BoxFusenError;
    type Future = FusenFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, req: Request<hyper::body::Incoming>) -> Self::Future {
        let fusen_filter = self.fusen_filter.clone();
        let http_codec = self.http_codec.clone();
        Box::pin(async move {
            Ok(match Self::call(req, http_codec, fusen_filter).await {
                Ok(response) => response,
                Err(fusen_error) => {
                    let mut status = 500;
                    if let FusenError::NotFind(_) = fusen_error {
                        status = 404;
                    }
                    Response::builder()
                        .status(status)
                        .body(get_empty_body())
                        .unwrap()
                }
            })
        })
    }
}
