use super::{
    Readiness,
    routes::{MatchedRoute, RouteTable, validate_query_pairs},
};
use crate::{
    RpcCategory, RpcContext, RpcError,
    context::RpcContextParts,
    middleware::{Next, Terminal},
    runtime::{
        BoxFuture,
        admission::{AdmissionError, AdmissionGate, AdmissionGuard},
        budget::ByteBudget,
        deadline::Deadline,
        metrics::SafeMetrics,
    },
    service::ServerInvocation,
    wire::{
        self, GuardedBody, RequestControl, decode_fusen_request, encode_problem, encode_success,
        parse_content_length, parse_request_control, read_body, validate_attempt,
        validate_content_type, validate_protocol_version,
    },
};
use fusen_contract::{ProtocolSet, WireProtocol};
use fusen_observability::{MetricEvent, MetricOutcome, MetricSide};
use futures_util::FutureExt;
use http::{
    HeaderMap, Request, Response, StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
};
use hyper::{body::Incoming, service::Service};
use serde_json::Value;
use std::{
    convert::Infallible,
    future::Future,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant as StdInstant},
};
use tokio::sync::Semaphore;
use tracing::Instrument;

#[derive(Clone)]
pub(crate) struct HttpApp {
    routes: Arc<RouteTable>,
    readiness: Arc<Readiness>,
    protocols: ProtocolSet,
    request_timeout: Duration,
    max_uri_bytes: usize,
    max_query_pairs: usize,
    max_headers: usize,
    max_header_bytes: usize,
    max_request_body: usize,
    max_response_body: usize,
    admission: Arc<AdmissionGate>,
    queue_slots: Option<Arc<Semaphore>>,
    queue_max_wait: Duration,
    request_budget: Arc<ByteBudget>,
    response_budget: Arc<ByteBudget>,
    metrics: SafeMetrics,
}

pub(crate) struct HttpAppConfig {
    pub protocols: ProtocolSet,
    pub request_timeout: Duration,
    pub max_uri_bytes: usize,
    pub max_query_pairs: usize,
    pub max_headers: usize,
    pub max_header_bytes: usize,
    pub max_request_body: usize,
    pub max_response_body: usize,
    pub max_concurrent_requests: usize,
    pub queue_capacity: usize,
    pub queue_max_wait: Duration,
    pub request_byte_budget: usize,
    pub response_byte_budget: usize,
}

impl HttpApp {
    pub(crate) fn new(
        routes: Arc<RouteTable>,
        readiness: Arc<Readiness>,
        config: HttpAppConfig,
        metrics: SafeMetrics,
    ) -> Self {
        Self {
            routes,
            readiness,
            protocols: config.protocols,
            request_timeout: config.request_timeout,
            max_uri_bytes: config.max_uri_bytes,
            max_query_pairs: config.max_query_pairs,
            max_headers: config.max_headers,
            max_header_bytes: config.max_header_bytes,
            max_request_body: config.max_request_body,
            max_response_body: config.max_response_body,
            admission: AdmissionGate::new(config.max_concurrent_requests),
            queue_slots: (config.queue_capacity > 0)
                .then(|| Arc::new(Semaphore::new(config.queue_capacity))),
            queue_max_wait: config.queue_max_wait,
            request_budget: ByteBudget::new(config.request_byte_budget),
            response_budget: ByteBudget::new(config.response_byte_budget),
            metrics,
        }
    }

    pub(crate) fn begin_draining(&self) {
        self.admission.begin_draining();
    }

    pub(crate) async fn drained(&self) {
        self.admission.drained().await;
    }

    async fn handle(&self, request: Request<Incoming>) -> Response<GuardedBody> {
        let fallback_request_id = uuid::Uuid::new_v4().simple().to_string();
        match self.try_handle(request).await {
            Ok(response) => response,
            Err((error, request_id, instance)) => encode_problem(
                &error,
                request_id.as_deref().unwrap_or(&fallback_request_id),
                instance,
            ),
        }
    }

    async fn try_handle(
        &self,
        request: Request<Incoming>,
    ) -> Result<Response<GuardedBody>, (RpcError, Option<String>, Option<String>)> {
        let path = request.uri().path().to_owned();
        let instance = Some(path.clone());
        let mut known_request_id = None;
        let result = self.try_handle_inner(request, &mut known_request_id).await;
        result.map_err(|error| (error, known_request_id, instance))
    }

    async fn try_handle_inner(
        &self,
        request: Request<Incoming>,
        known_request_id: &mut Option<String>,
    ) -> Result<Response<GuardedBody>, RpcError> {
        self.validate_head(&request)?;
        let path = request.uri().path().to_owned();
        let protocol = if request.uri().path().starts_with("/_fusen/v1/") {
            WireProtocol::FusenV1
        } else {
            WireProtocol::SpringCloudV1
        };
        if !self.protocols.contains(protocol) {
            return Err(RpcError::framework(
                RpcCategory::NotFound,
                "protocol_not_enabled",
                "wire protocol is not enabled by this server",
            ));
        }
        validate_protocol_version(protocol, request.version())?;
        let control = parse_request_control(request.headers(), self.request_timeout)?;
        *known_request_id = Some(control.request_id.clone());
        if control.deadline.is_elapsed() {
            return Err(deadline_exceeded());
        }
        match self.readiness.load() {
            super::NOT_READY => {
                return Err(RpcError::framework(
                    RpcCategory::Unavailable,
                    "not_ready",
                    "server has not completed startup",
                ));
            }
            super::DRAINING | super::STOPPED => {
                return Err(RpcError::framework(
                    RpcCategory::Unavailable,
                    "draining",
                    "server is draining",
                )
                .mark_retryable());
            }
            super::READY => {}
            _ => unreachable!("validated readiness state"),
        }

        let matched = match protocol {
            WireProtocol::FusenV1 => self
                .routes
                .match_fusen(request.uri().path(), request.headers())?,
            WireProtocol::SpringCloudV1 => self
                .routes
                .match_spring(request.method(), request.uri().path())?,
            _ => {
                return Err(RpcError::framework(
                    RpcCategory::Unimplemented,
                    "unsupported_wire_protocol",
                    "server does not support this wire protocol",
                ));
            }
        };
        if protocol == WireProtocol::SpringCloudV1 {
            validate_query_pairs(request.uri().query(), self.max_query_pairs)?;
        }
        validate_attempt(control.attempt, matched.route.method.idempotency())?;
        let _admission = self.acquire_admission(control.deadline).await?;

        let started = StdInstant::now();
        self.metrics.record(&MetricEvent::InvocationStarted {
            side: MetricSide::Server,
            protocol: protocol.as_str(),
            service: matched.route.service.selector().service_id(),
            method: matched.route.method.fusen_identity(),
        });
        let span = tracing::info_span!(
            "fusen.server.invocation",
            request_id = %control.request_id,
            protocol = protocol.as_str(),
            service = matched.route.service.selector().service_id(),
            method = matched.route.method.fusen_identity(),
            attempt = control.attempt,
        );
        let processed = AssertUnwindSafe(
            self.execute_matched(request, &matched, protocol, &control),
        )
        .catch_unwind()
        .instrument(span)
        .await
        .unwrap_or_else(|_| {
            tracing::error!(request_id = %control.request_id, "server request processing panicked");
            Err(request_panicked())
        });
        let (outcome, error_code, response) = match processed {
            Ok(response) => (MetricOutcome::Success, None, response),
            Err(error) => {
                let outcome = match error.category() {
                    RpcCategory::DeadlineExceeded => MetricOutcome::Timeout,
                    RpcCategory::Cancelled => MetricOutcome::Cancelled,
                    RpcCategory::ResourceExhausted => MetricOutcome::Rejected,
                    _ => MetricOutcome::Error,
                };
                let error_code = error.code().as_str().to_owned();
                let response = encode_problem(&error, &control.request_id, Some(path));
                (outcome, Some(error_code), response)
            }
        };
        self.metrics.record(&MetricEvent::InvocationFinished {
            side: MetricSide::Server,
            protocol: protocol.as_str(),
            service: matched.route.service.selector().service_id(),
            method: matched.route.method.fusen_identity(),
            outcome,
            status_class: Some(status_class(response.status())),
            error_code: error_code.as_deref(),
            duration: started.elapsed(),
            attempts: control.attempt,
        });
        let mut response = response;
        response.headers_mut().insert(
            wire::REQUEST_ID,
            http::HeaderValue::from_str(&control.request_id)
                .expect("validated request ID is a valid header value"),
        );
        Ok(response)
    }

    async fn execute_matched(
        &self,
        request: Request<Incoming>,
        matched: &MatchedRoute,
        protocol: WireProtocol,
        control: &RequestControl,
    ) -> Result<Response<GuardedBody>, RpcError> {
        let request_headers = application_headers(request.headers());
        let query = request.uri().query().map(str::to_owned);
        let content_length = parse_content_length(request.headers())?;
        let body_required = protocol == WireProtocol::FusenV1 || matched.spring_has_body();
        validate_content_type(
            request.headers(),
            match protocol {
                WireProtocol::FusenV1 => wire::FUSEN_CONTENT_TYPE,
                WireProtocol::SpringCloudV1 => wire::JSON_CONTENT_TYPE,
                _ => {
                    return Err(RpcError::framework(
                        RpcCategory::Unimplemented,
                        "unsupported_wire_protocol",
                        "server does not support this wire protocol",
                    ));
                }
            },
            body_required,
        )?;
        if !body_required && content_length.is_some_and(|length| length > 0) {
            return Err(RpcError::framework(
                RpcCategory::InvalidArgument,
                "unexpected_body",
                "this SpringCloudV1 route does not accept a request body",
            ));
        }
        let (_parts, body) = request.into_parts();
        let arguments = if body_required {
            let (bytes, body_permit) = control
                .deadline
                .run(read_body(
                    body,
                    content_length,
                    self.max_request_body,
                    &self.request_budget,
                ))
                .await
                .map_err(|_| deadline_exceeded())??;
            let arguments = match protocol {
                WireProtocol::FusenV1 => decode_fusen_request(&bytes)?,
                WireProtocol::SpringCloudV1 => {
                    let body = serde_json::from_slice::<Value>(&bytes).map_err(|_| {
                        RpcError::framework(
                            RpcCategory::InvalidArgument,
                            "invalid_json",
                            "SpringCloudV1 request body is invalid JSON",
                        )
                    })?;
                    matched.spring_arguments(query.as_deref(), Some(body), self.max_query_pairs)?
                }
                _ => {
                    return Err(RpcError::framework(
                        RpcCategory::Unimplemented,
                        "unsupported_wire_protocol",
                        "server does not support this wire protocol",
                    ));
                }
            };
            drop(body_permit);
            arguments
        } else {
            matched.spring_arguments(query.as_deref(), None, self.max_query_pairs)?
        };

        let context = RpcContext::new(RpcContextParts {
            request_id: control.request_id.clone(),
            protocol,
            service: matched.route.service,
            method: matched.route.method,
            deadline: control.deadline,
            attempt: control.attempt,
            headers: request_headers,
            arguments,
            response_limit: self.max_response_body,
            response_wire_overhead: match protocol {
                WireProtocol::FusenV1 => 11,
                WireProtocol::SpringCloudV1 => 0,
                _ => unreachable!("the protocol was validated before dispatch"),
            },
            response_budget: self.response_budget.clone(),
        });
        let terminal = ServiceTerminal {
            dispatch: matched.route.dispatch.as_ref(),
            max_response_body: self.max_response_body,
            response_budget: self.response_budget.clone(),
        };
        let response = control
            .deadline
            .run(Next::new(&matched.route.middleware, &terminal).run(context))
            .await
            .map_err(|_| deadline_exceeded())??;
        encode_success(
            protocol,
            response,
            self.max_response_body,
            &self.response_budget,
        )
    }

    fn validate_head(&self, request: &Request<Incoming>) -> Result<(), RpcError> {
        if request.uri().to_string().len() > self.max_uri_bytes {
            return Err(RpcError::framework(
                RpcCategory::InvalidArgument,
                "uri_too_large",
                "request URI exceeds the configured limit",
            ));
        }
        if request.headers().len() > self.max_headers {
            return Err(RpcError::framework(
                RpcCategory::InvalidArgument,
                "too_many_headers",
                "request contains too many headers",
            ));
        }
        let bytes = request
            .headers()
            .iter()
            .try_fold(0usize, |total, (name, value)| {
                total
                    .checked_add(name.as_str().len())
                    .and_then(|total| total.checked_add(value.as_bytes().len()))
            });
        if bytes.is_none_or(|bytes| bytes > self.max_header_bytes) {
            return Err(RpcError::framework(
                RpcCategory::InvalidArgument,
                "headers_too_large",
                "request headers exceed the configured byte limit",
            ));
        }
        Ok(())
    }

    async fn acquire_admission(&self, deadline: Deadline) -> Result<AdmissionGuard, RpcError> {
        match self.admission.try_enter() {
            Ok(guard) => return Ok(guard),
            Err(AdmissionError::Draining) => return Err(draining()),
            Err(AdmissionError::Overloaded) => {}
        }
        let Some(queue) = &self.queue_slots else {
            self.metrics.record(&MetricEvent::AdmissionRejected {
                side: MetricSide::Server,
                reason: "concurrency",
            });
            return Err(overloaded());
        };
        let queue_permit = queue
            .clone()
            .try_acquire_owned()
            .map_err(|_| overloaded())?;
        let queue_deadline = deadline.min(Deadline::after(self.queue_max_wait));
        let result = queue_deadline.run(self.admission.enter()).await;
        drop(queue_permit);
        match result {
            Ok(Ok(guard)) => Ok(guard),
            Ok(Err(AdmissionError::Draining)) => Err(draining()),
            Ok(Err(AdmissionError::Overloaded)) => Err(overloaded()),
            Err(_) if deadline.is_elapsed() => Err(deadline_exceeded()),
            Err(_) => Err(RpcError::framework(
                RpcCategory::ResourceExhausted,
                "admission_queue_timeout",
                "request did not enter admission before the queue wait limit",
            )),
        }
    }
}

impl Service<Request<Incoming>> for HttpApp {
    type Response = Response<GuardedBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, request: Request<Incoming>) -> Self::Future {
        let app = self.clone();
        Box::pin(async move {
            let response = match AssertUnwindSafe(app.handle(request)).catch_unwind().await {
                Ok(response) => response,
                Err(_) => {
                    tracing::error!("server HTTP service panicked outside the invocation boundary");
                    let request_id = uuid::Uuid::new_v4().simple().to_string();
                    encode_problem(&request_panicked(), &request_id, None)
                }
            };
            Ok(response)
        })
    }
}

struct ServiceTerminal<'a> {
    dispatch: &'a dyn crate::service::ErasedDispatch,
    max_response_body: usize,
    response_budget: Arc<ByteBudget>,
}

impl Terminal for ServiceTerminal<'_> {
    fn call<'a>(&'a self, context: RpcContext) -> BoxFuture<'a, crate::RpcResult> {
        self.dispatch.call(ServerInvocation::new(
            context,
            self.max_response_body,
            self.response_budget.clone(),
        ))
    }
}

fn application_headers(headers: &HeaderMap) -> HeaderMap {
    let mut headers = headers.clone();
    for name in [
        CONTENT_TYPE,
        CONTENT_LENGTH,
        wire::REQUEST_ID,
        wire::TIMEOUT_MS,
        wire::ATTEMPT,
        wire::SERVICE_GROUP,
        wire::SERVICE_VERSION,
    ] {
        headers.remove(name);
    }
    headers
}

fn overloaded() -> RpcError {
    RpcError::framework(
        RpcCategory::ResourceExhausted,
        "overloaded",
        "server request concurrency is exhausted",
    )
}

fn draining() -> RpcError {
    RpcError::framework(RpcCategory::Unavailable, "draining", "server is draining").mark_retryable()
}

fn deadline_exceeded() -> RpcError {
    RpcError::framework(
        RpcCategory::DeadlineExceeded,
        "deadline_exceeded",
        "RPC deadline elapsed",
    )
}

fn request_panicked() -> RpcError {
    RpcError::framework(
        RpcCategory::Internal,
        "request_panic",
        "request processing failed",
    )
}

fn status_class(status: StatusCode) -> &'static str {
    match status.as_u16() / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        _ => "5xx",
    }
}
