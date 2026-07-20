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
use http::{HeaderValue, Request, Response, StatusCode, header::CONTENT_TYPE};
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
            let request_id = request_identifier(&request);
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
            let result = tokio::time::timeout(
                router.request_timeout,
                route(request, router, request_id.clone()),
            )
            .await;
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
    request_id: String,
) -> Result<Response<BoxBody<Bytes, Infallible>>, FusenError> {
    let mut request: FusenRequest =
        RequestCodec::decode(&router.http_codec, request.map(|body| body.boxed())).await?;
    let QueryResult {
        method_info,
        path_parameters,
    } = router
        .path_cache
        .search(&request.path)?
        .ok_or_else(|| FusenError::RouteNotFound(request.path.path.clone()))?;
    request.path_parameters = path_parameters;
    let protocol = request.protocol;
    let context = FusenContext {
        unique_identifier: request_id.clone(),
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
    response.headers.insert(
        "x-request-id",
        HeaderValue::from_str(&request_id)
            .map_err(|error| FusenError::internal("failed to create request ID header", error))?,
    );
    ResponseCodec::encode(&router.http_codec, &mut response)
}

fn request_identifier(request: &Request<hyper::body::Incoming>) -> String {
    request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
        .map(str::to_owned)
        .unwrap_or_else(uuid)
}

fn problem_response(
    error: FusenError,
    request_id: String,
    instance: Option<String>,
) -> Response<BoxBody<Bytes, Infallible>> {
    if matches!(error, FusenError::Internal { .. }) {
        tracing::error!(%request_id, error = ?error, "request failed");
    }
    let request_id_header = HeaderValue::from_str(&request_id).ok();
    let requested_status = error.status();
    let status = if requested_status.is_client_error() || requested_status.is_server_error() {
        requested_status
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    let mut problem = error.problem_details(request_id, instance);
    if status != requested_status {
        problem.status = status.as_u16();
        problem.title = "Internal Server Error".into();
        problem.code = "internal_error".into();
        problem.type_uri = "https://fusen.rs/problems/internal_error".into();
        problem.detail = Some("Internal server error".into());
    }
    let body = serde_json::to_vec(&problem)
        .unwrap_or_else(|_| b"{\"title\":\"Internal Server Error\",\"status\":500}".to_vec());
    let mut builder = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/problem+json");
    if let Some(request_id) = &request_id_header {
        builder = builder.header("x-request-id", request_id);
    }
    let response = builder.body(Full::new(Bytes::from(body)).boxed());
    match response {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(?error, "failed to build problem response");
            let mut response = Response::new(
                Full::new(Bytes::from_static(
                    b"{\"title\":\"Internal Server Error\",\"status\":500}",
                ))
                .boxed(),
            );
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/problem+json"),
            );
            if let Some(request_id) = request_id_header {
                response.headers_mut().insert("x-request-id", request_id);
            }
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_response_preserves_request_id_and_error_status() {
        let response = problem_response(
            FusenError::InvalidRequest("bad input".into()),
            "request-123".into(),
            Some("/demo".into()),
        );
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()["x-request-id"], "request-123");
    }

    #[test]
    fn every_problem_response_has_an_error_status() {
        let response = problem_response(
            FusenError::Remote(Box::new(crate::error::ProblemDetails {
                type_uri: "about:blank".into(),
                title: "invalid".into(),
                status: 200,
                detail: None,
                instance: None,
                code: "invalid".into(),
                request_id: "remote".into(),
            })),
            "request-123".into(),
            None,
        );
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
