use bytes::Bytes;
use fusen_common::{
    error::{BoxFusenError, FusenError},
    FusenFuture,
};
use http_body_util::{combinators::BoxBody, BodyExt};
use hyper::{service::Service, Request, Response};
use std::{convert::Infallible, sync::Arc};

use crate::{
    codec::{http_codec::FusenHttpCodec, HttpCodec},
    filter::FusenFilter,
    get_empty_body,
};

#[derive(Clone)]
pub struct FusenRouter<KF: 'static> {
    fusen_filter: &'static KF,
    http_codec: Arc<FusenHttpCodec>,
}

impl<KF> FusenRouter<KF>
where
    KF: FusenFilter,
{
    pub fn new(fusen_filter: &'static KF) -> Self {
        FusenRouter {
            fusen_filter,
            http_codec: Arc::new(FusenHttpCodec::new()),
        }
    }

    async fn call(
        request: Request<hyper::body::Incoming>,
        http_codec: Arc<FusenHttpCodec>,
        fusen_filter: &'static KF,
    ) -> Result<Response<BoxBody<Bytes, Infallible>>, FusenError> {
        let request = request.map(|e| e.boxed());
        let context = http_codec.as_ref().decode(request).await?;
        let context = fusen_filter.call(context).await?;
        let response = http_codec.encode(context).await?;
        Ok(response)
    }
}

impl<KF> Service<Request<hyper::body::Incoming>> for FusenRouter<KF>
where
    KF: FusenFilter + Clone + Send + 'static + Sync,
{
    type Response = Response<BoxBody<Bytes, Infallible>>;
    type Error = BoxFusenError;
    type Future = FusenFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, req: Request<hyper::body::Incoming>) -> Self::Future {
        let fusen_filter = self.fusen_filter;
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
