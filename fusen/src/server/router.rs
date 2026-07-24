use crate::{
    error::FusenError,
    invocation::{
        InvocationGuard, InvocationObserver, InvocationPhase, InvocationSide, PhaseTracker,
        TargetTracker,
    },
    protocol::{
        codec::{FusenHttpCodec, RequestCodec, ResponseCodec},
        fusen::{context::RpcContext, request::FusenRequest},
    },
    server::{
        path::{PathCache, QueryResult},
        rpc::RouteDispatch,
    },
};
use bytes::Bytes;
use fusen_contract::StaticBoxFuture;
use http::{HeaderValue, Request, Response, StatusCode, header::CONTENT_TYPE};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use hyper::service::Service;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(crate) struct HttpRouter {
    pub(crate) context: Arc<RouterContext>,
}

pub(crate) struct RouterContext {
    pub(crate) http_codec: FusenHttpCodec,
    pub(crate) path_cache: PathCache,
    pub(crate) dispatches: Arc<[RouteDispatch]>,
    pub(crate) observers: Arc<[Arc<dyn InvocationObserver>]>,
    pub(crate) concurrency: Arc<Semaphore>,
    pub(crate) request_timeout: Duration,
}

impl Service<Request<hyper::body::Incoming>> for HttpRouter {
    type Response = Response<BoxBody<Bytes, Infallible>>;
    type Error = Infallible;
    type Future = StaticBoxFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, request: Request<hyper::body::Incoming>) -> Self::Future {
        let router = self.context.clone();
        Box::pin(async move {
            let request_id = request_identifier(&request);
            let instance = Some(request.uri().path().to_owned());
            let deadline = tokio::time::Instant::now() + router.request_timeout;
            let mut guard = InvocationGuard::start(
                &router.observers,
                InvocationSide::Server,
                &request_id,
                None,
                None,
            );
            let tracker = guard.tracker();
            let target = guard.target_tracker();
            let permit = match router.concurrency.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    let error =
                        FusenError::ServiceUnavailable("server concurrency limit reached".into());
                    guard.finish_error(&error);
                    return Ok(problem_response(error, request_id, instance));
                }
            };
            let result = tokio::time::timeout_at(
                deadline,
                route(
                    request,
                    router.clone(),
                    &request_id,
                    deadline,
                    tracker,
                    target,
                ),
            )
            .await;
            drop(permit);
            Ok(match result {
                Ok(Ok(response)) => {
                    guard.finish_response(response.status());
                    response
                }
                Ok(Err(error)) => {
                    guard.finish_error(&error);
                    problem_response(error, request_id, instance)
                }
                Err(_) => {
                    guard.finish_timeout();
                    problem_response(
                        FusenError::Timeout("server request deadline exceeded".into()),
                        request_id,
                        instance,
                    )
                }
            })
        })
    }
}

async fn route(
    request: Request<hyper::body::Incoming>,
    router: Arc<RouterContext>,
    request_id: &str,
    deadline: tokio::time::Instant,
    tracker: PhaseTracker,
    target: TargetTracker,
) -> Result<Response<BoxBody<Bytes, Infallible>>, FusenError> {
    tracker.set(InvocationPhase::Decode);
    let mut request: FusenRequest =
        RequestCodec::decode(&router.http_codec, request.map(|body| body.boxed())).await?;
    tracker.set(InvocationPhase::Route);
    let QueryResult {
        route_index,
        service,
        method,
        path_parameters,
    } = router
        .path_cache
        .search(&request.path)?
        .ok_or_else(|| FusenError::RouteNotFound(request.path.path.clone()))?;
    request.path_parameters = path_parameters;
    let protocol = request.protocol;
    target.set(service.selector().service_id(), method.name());
    let context = RpcContext::new(request_id.to_owned(), service, method, request, deadline);
    let dispatch = router.dispatches.get(route_index).ok_or_else(|| {
        FusenError::internal(
            "route dispatch is missing",
            std::io::Error::other("invalid route dispatch index"),
        )
    })?;
    let mut response = dispatch.call(context, tracker.clone()).await?;
    response.protocol = protocol;
    response.headers.insert(
        "x-request-id",
        HeaderValue::from_str(request_id)
            .map_err(|error| FusenError::internal("failed to create request ID header", error))?,
    );
    tracker.set(InvocationPhase::Encode);
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
        .unwrap_or_else(crate::request_id::new_request_id)
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
