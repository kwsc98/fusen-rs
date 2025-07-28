use std::{convert::Infallible, sync::Arc};

use bytes::Bytes;
use fusen_internal_common::{BoxFuture, utils::uuid::uuid};
use http::{Request, Response};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::service::Service;

use crate::{
    error::FusenError,
    handler::HandlerContext,
    protocol::{
        codec::{FusenHttpCodec, RequestCodec, ResponseCodec},
        fusen::{
            context::FusenContext, metadata::MetaData, request::FusenRequest,
            response::FusenResponse,
        },
    },
    server::path::{PathCache, QueryResult},
};

#[derive(Clone)]
pub struct Router {
    http_codec: Arc<FusenHttpCodec>,
    path_cache: Arc<PathCache>,
    handler_context: Arc<HandlerContext>,
}

impl Service<Request<hyper::body::Incoming>> for Router {
    type Response = Response<BoxBody<Bytes, Infallible>>;

    type Error = FusenError;

    type Future = BoxFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, request: Request<hyper::body::Incoming>) -> Self::Future {
        let router = self.clone();
        Box::pin(async move {
            //首先进行编解码
            let request = request.map(|e| e.boxed());
            let mut fusen_request: FusenRequest =
                RequestCodec::decode(router.http_codec.as_ref(), request).await?;
            //通过path找到资源
            let Some(QueryResult {
                method_info,
                rest_fields,
            }) = router.path_cache.seach(&fusen_request.path).await
            else {
                return Response::builder()
                    .status(404)
                    .body(Full::new(Bytes::new()).boxed())
                    .map_err(|error| FusenError::Error(Box::new(error)));
            };
            if let Some(rest_fields) = rest_fields {
                for (key, value) in rest_fields {
                    fusen_request.querys.insert(key, value);
                }
            }
            let mut context = FusenContext {
                unique_identifier: uuid(),
                metadata: MetaData::default(),
                method_info,
                request: fusen_request,
                response: FusenResponse::default(),
            };
            //通过service获取handler


            let response =
                ResponseCodec::encode(router.http_codec.as_ref(), &mut context.response)?;
            Ok(response)
        })
    }
}
