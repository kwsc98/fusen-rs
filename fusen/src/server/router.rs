use crate::{
    error::FusenError,
    filter::ProceedingJoinPoint,
    handler::HandlerContext,
    protocol::{
        codec::{FusenHttpCodec, RequestCodec, ResponseCodec},
        fusen::{
            context::FusenContext, metadata::MetaData, request::FusenRequest,
            response::FusenResponse,
        },
    },
    server::{
        path::{PathCache, QueryResult},
        rpc::RpcServerHandler,
    },
};
use bytes::Bytes;
use fusen_internal_common::{BoxFuture, utils::uuid::uuid};
use http::{Request, Response};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::service::Service;
use std::{convert::Infallible, sync::Arc};

#[derive(Clone)]
pub struct Router {
    http_codec: Arc<FusenHttpCodec>,
    path_cache: Arc<PathCache>,
    handler_context: Arc<HandlerContext>,
    fusen_service_handler: Arc<RpcServerHandler>,
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
            let context = FusenContext {
                unique_identifier: uuid(),
                metadata: MetaData::default(),
                method_info,
                request: fusen_request,
                response: FusenResponse::default(),
            };
            //通过service获取handler
            let handler_controller = router
                .handler_context
                .get_controller(&context.method_info.service_desc);
            let aspect_handers = handler_controller.aspect.clone();
            let join_point = ProceedingJoinPoint::new(aspect_handers, context);
            let mut context = router.fusen_service_handler.call(join_point).await?;
            let response =
                ResponseCodec::encode(router.http_codec.as_ref(), &mut context.response)?;
            Ok(response)
        })
    }
}
