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
use http::{Request, Response, header::CONTENT_TYPE};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::service::Service;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub struct Router {
    pub context: Arc<RouterContext>,
}

pub struct RouterContext {
    pub http_codec: FusenHttpCodec,
    pub path_cache: PathCache,
    pub handler_context: HandlerContext,
    pub fusen_service_handler: RpcServerHandler,
    pub concurrency: Arc<Semaphore>,
    pub request_timeout: Duration,
}

impl Service<Request<hyper::body::Incoming>> for Router {
    type Response = Response<BoxBody<Bytes, Infallible>>;
    type Error = Infallible;
    type Future = BoxFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, request: Request<hyper::body::Incoming>) -> Self::Future {
        let router = self.context.clone();
        Box::pin(async move {
            let request_id = uuid();
            let instance = Some(request.uri().path().to_owned());
            let permit = match router.concurrency.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    return Ok(problem_response(
                        FusenError::ServiceUnavailable("server concurrency limit reached".into()),
                        request_id,
                        instance,
                    ));
                }
            };
            let result = tokio::time::timeout(router.request_timeout, route(request, router)).await;
            drop(permit);
            Ok(match result {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => problem_response(error, request_id, instance),
                Err(_) => problem_response(
                    FusenError::Timeout("server request deadline exceeded".into()),
                    request_id,
                    instance,
                ),
            })
        })
    }
}

async fn route(
    request: Request<hyper::body::Incoming>,
    router: Arc<RouterContext>,
) -> Result<Response<BoxBody<Bytes, Infallible>>, FusenError> {
    let mut request: FusenRequest =
        RequestCodec::decode(&router.http_codec, request.map(|body| body.boxed())).await?;
    let QueryResult {
        method_info,
        rest_fields,
    } = router
        .path_cache
        .search(&request.path)
        .ok_or_else(|| FusenError::RouteNotFound(request.path.path.clone()))?;
    if let Some(fields) = rest_fields {
        request.querys.extend(fields);
    }
    let protocol = request.protocol;
    let context = FusenContext {
        unique_identifier: uuid(),
        metadata: MetaData::default(),
        method_info,
        request,
        response: None,
    };
    let controller = router
        .handler_context
        .get_controller(&context.method_info.service_desc)?;
    let context = router
        .fusen_service_handler
        .call(controller.aspect.clone(), context)
        .await?;
    let mut response = context.response.ok_or_else(|| FusenError::Internal {
        message: "service handler returned no response",
        source: Box::new(std::io::Error::other("missing response invariant")),
    })?;
    response.protocol = protocol;
    ResponseCodec::encode(&router.http_codec, &mut response)
}

fn problem_response(
    error: FusenError,
    request_id: String,
    instance: Option<String>,
) -> Response<BoxBody<Bytes, Infallible>> {
    if matches!(error, FusenError::Internal { .. }) {
        tracing::error!(%request_id, error = ?error, "request failed");
    }
    let problem = error.problem_details(request_id, instance);
    let status = problem.status;
    let body = serde_json::to_vec(&problem)
        .unwrap_or_else(|_| b"{\"title\":\"Internal Server Error\",\"status\":500}".to_vec());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/problem+json")
        .body(Full::new(Bytes::from(body)).boxed())
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new()).boxed()))
}
