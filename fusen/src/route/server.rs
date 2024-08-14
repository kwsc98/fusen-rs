use bytes::Bytes;
use fusen_common::{
    error::{BoxFusenError, FusenError},
    FusenFuture,
};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{service::Service, Request, Response};
use std::{convert::Infallible, sync::Arc};

use crate::{
    codec::{http_codec::FusenHttpCodec, HttpCodec},
    filter::FusenFilter,
    handler::HandlerContext,
};

#[derive(Clone)]
pub struct FusenRouter<KF: 'static> {
    fusen_filter: &'static KF,
    http_codec: Arc<FusenHttpCodec>,
    handler_context: Arc<HandlerContext>,
}

impl<KF> FusenRouter<KF>
where
    KF: FusenFilter,
{
    pub fn new(
        fusen_filter: &'static KF,
        http_codec: Arc<FusenHttpCodec>,
        handler_context: Arc<HandlerContext>,
    ) -> Self {
        FusenRouter {
            fusen_filter,
            http_codec,
            handler_context,
        }
    }

    async fn call(
        request: Request<hyper::body::Incoming>,
        http_codec: Arc<FusenHttpCodec>,
        fusen_filter: &'static KF,
        handler_context: Arc<HandlerContext>,
    ) -> Result<Response<BoxBody<Bytes, Infallible>>, FusenError> {
        let request = request.map(|e| e.boxed());
        let context = http_codec.decode(request).await?;
        let handler = handler_context
            .get_controller(&context.context_info.get_handler_key())
            .get_aspect();
        let context = handler.aroud_(fusen_filter, context).await?;
        let response = http_codec.encode(context).await?;
        Ok(response)
    }
}

impl<KF> Service<Request<hyper::body::Incoming>> for FusenRouter<KF>
where
    KF: FusenFilter,
{
    type Response = Response<BoxBody<Bytes, Infallible>>;
    type Error = BoxFusenError;
    type Future = FusenFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, req: Request<hyper::body::Incoming>) -> Self::Future {
        let fusen_filter = self.fusen_filter;
        let http_codec = self.http_codec.clone();
        let handler_context = self.handler_context.clone();

        Box::pin(async move {
            Ok(
                match Self::call(req, http_codec, fusen_filter, handler_context).await {
                    Ok(response) => response,
                    Err(fusen_error) => {
                        let mut status = 500;
                        if let FusenError::NotFind = fusen_error {
                            status = 404;
                        }
                        Response::builder()
                            .status(status)
                            .body(
                                Full::new(Bytes::copy_from_slice(
                                    format!("{:?}", fusen_error).as_bytes(),
                                ))
                                .boxed(),
                            )
                            .unwrap()
                    }
                },
            )
        })
    }
}
