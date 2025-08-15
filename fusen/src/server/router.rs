use crate::{
    error::FusenError,
    handler::HandlerContext,
    protocol::{
        codec::{FusenHttpCodec, RequestCodec, ResponseCodec},
        fusen::{context::FusenContext, metadata::MetaData, request::FusenRequest},
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
    pub context: Arc<RouterContext>,
}

pub struct RouterContext {
    pub http_codec: FusenHttpCodec,
    pub path_cache: PathCache,
    pub handler_context: HandlerContext,
    pub fusen_service_handler: RpcServerHandler,
}

impl Service<Request<hyper::body::Incoming>> for Router {
    type Response = Response<BoxBody<Bytes, Infallible>>;

    type Error = FusenError;

    type Future = BoxFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, request: Request<hyper::body::Incoming>) -> Self::Future {
        let router = self.context.clone();
        Box::pin(async move {
            let result = call(request, router).await;
            match result {
                Ok(response) => Ok(response),
                Err(error) => Ok(error.into()),
            }
        })
    }
}

async fn call(
    request: Request<hyper::body::Incoming>,
    router: Arc<RouterContext>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, FusenError> {
    //首先进行编解码
    let request = request.map(|e| e.boxed());
    let mut fusen_request: FusenRequest = RequestCodec::decode(&router.http_codec, request).await?;
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
        response: Default::default(),
    };
    //通过service获取handler
    let handler_controller = router
        .handler_context
        .get_controller(&context.method_info.service_desc);
    let aspect_handers = handler_controller.aspect.clone();
    let context = router
        .fusen_service_handler
        .call(aspect_handers, context)
        .await?;
    let response = ResponseCodec::encode(&router.http_codec, &mut context.response.unwrap())?;
    Ok(response)
}

impl From<FusenError> for Response<BoxBody<Bytes, Infallible>> {
    fn from(error: FusenError) -> Self {
        let mut builder = Response::builder();
        let mut body = Bytes::new();
        match error {
            FusenError::HttpError(http_status) => {
                builder = builder.status(http_status.status);
                if let Some(message) = http_status.message {
                    body = Bytes::copy_from_slice(message.as_bytes());
                }
            }
            _error => {
                builder = builder.status(500);
            }
        }
        builder.body(Full::new(body).boxed()).unwrap()
    }
}
