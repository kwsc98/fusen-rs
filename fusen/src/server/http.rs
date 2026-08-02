use super::{
    Readiness,
    routes::{MatchedRoute, RouteTable, validate_query_pairs},
};
use crate::{
    Context, Error, ErrorCategory, InterceptionStage, RetryHint, Side,
    context::ContextParts,
    interceptor::{Next, Terminal},
    runtime::{
        admission::{AdmissionError, AdmissionGate, AdmissionGuard},
        budget::ByteBudget,
        deadline::Deadline,
        metrics::SafeMetrics,
    },
    service::ServerInvocation,
    wire::{
        self, GuardedBody, RequestControl, encode_problem, encode_success, parse_content_length,
        parse_request_control, read_body, validate_attempt, validate_content_type,
        validate_http_version, validated_request_id_header,
    },
};
use bytes::Bytes;
use fusen_contract::{HTTP_JSON_V1, HttpBindingId, HttpVersionSet};
use fusen_observability::{
    AdmissionRejectedEvent, InvocationFinishedEvent, InvocationStartedEvent, MetricEvent,
    MetricOutcome, MetricSide,
};
use futures_util::FutureExt;
use http::{
    HeaderMap, Request, Response as HttpResponse, StatusCode,
    header::{
        ACCEPT, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST, TE, TRAILER, TRANSFER_ENCODING,
        UPGRADE,
    },
};
use hyper::{
    body::{Body as HttpBody, Incoming},
    service::Service,
};
use serde_json::Value;
use std::{
    convert::Infallible,
    future::Future,
    panic::AssertUnwindSafe,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant as StdInstant},
};
use tokio::sync::Semaphore;
use tracing::Instrument;

#[derive(Clone)]
pub(crate) struct HttpApp {
    routes: Arc<RouteTable>,
    readiness: Arc<Readiness>,
    http_versions: HttpVersionSet,
    invocation_controls: bool,
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
    pub http_versions: HttpVersionSet,
    pub invocation_controls: bool,
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
            http_versions: config.http_versions,
            invocation_controls: config.invocation_controls,
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

    async fn handle(&self, request: Request<Incoming>) -> HttpResponse<GuardedBody> {
        let is_head = request.method() == http::Method::HEAD;
        let fallback_request_id = uuid::Uuid::new_v4().simple().to_string();
        let mut response = match self.try_handle(request).await {
            Ok(response) => response,
            Err((error, request_id, instance, controls)) => encode_problem(
                &error,
                request_id.as_deref().unwrap_or(&fallback_request_id),
                instance,
                controls,
            ),
        };
        if is_head {
            *response.body_mut() = GuardedBody::new(Bytes::new(), None);
        }
        response
    }

    async fn try_handle(
        &self,
        request: Request<Incoming>,
    ) -> Result<HttpResponse<GuardedBody>, (Error, Option<String>, Option<String>, bool)> {
        let path = request.uri().path().to_owned();
        let instance = Some(path.clone());
        let mut known_request_id = None;
        let mut controls = false;
        let result = self
            .try_handle_inner(request, &mut known_request_id, &mut controls)
            .await;
        result.map_err(|error| (error, known_request_id, instance, controls))
    }

    async fn try_handle_inner(
        &self,
        request: Request<Incoming>,
        known_request_id: &mut Option<String>,
        controls_negotiated: &mut bool,
    ) -> Result<HttpResponse<GuardedBody>, Error> {
        self.validate_head(&request)?;
        let path = request.uri().path().to_owned();
        validate_http_version(request.version())?;
        if !self.http_versions.contains(request.version()) {
            return Err(Error::framework(
                ErrorCategory::InvalidArgument,
                "http_version_not_enabled",
                "HTTP version is not enabled by this server",
            ));
        }
        let has_controls = has_invocation_controls(request.headers());
        if has_controls && !self.invocation_controls {
            return Err(Error::framework(
                ErrorCategory::InvalidArgument,
                "invocation_controls_not_supported",
                "endpoint does not support Fusen invocation control headers",
            ));
        }
        *controls_negotiated = has_controls;
        if has_controls {
            *known_request_id = validated_request_id_header(request.headers()).map(str::to_owned);
        }
        let control = parse_request_control(request.headers(), self.request_timeout)?;
        *known_request_id = Some(control.request_id.clone());
        if control.deadline.is_elapsed() {
            return Err(deadline_exceeded());
        }
        match self.readiness.load() {
            super::NOT_READY => {
                return Err(Error::framework(
                    ErrorCategory::Unavailable,
                    "not_ready",
                    "server has not completed startup",
                ));
            }
            super::DRAINING | super::STOPPED => {
                return Err(Error::framework(
                    ErrorCategory::Unavailable,
                    "draining",
                    "server is draining",
                )
                .with_retry_hint(RetryHint::Retryable));
            }
            super::READY => {}
            _ => unreachable!("validated readiness state"),
        }

        let matched = self
            .routes
            .match_http(request.method(), request.uri().path())?;
        if has_controls {
            validate_service_controls(
                request.headers(),
                matched.route.service.selector().group(),
                matched.route.service.selector().version(),
            )?;
        }
        validate_query_pairs(request.uri().query(), self.max_query_pairs)?;
        validate_attempt(control.attempt, matched.route.method.allows_retries())?;
        let _admission = self.acquire_admission(control.deadline).await?;

        let started = StdInstant::now();
        let http_version = request.version();
        self.metrics.record(&MetricEvent::InvocationStarted(
            InvocationStartedEvent::new(
                MetricSide::Server,
                HTTP_JSON_V1,
                Some(http_version_name(http_version)),
                matched.route.service.selector().service_id(),
                matched.route.method.invocation_name(),
            ),
        ));
        let span = tracing::info_span!(
            "fusen.server.invocation",
            request_id = %control.request_id,
            http_binding = HTTP_JSON_V1,
            network_protocol_version = ?request.version(),
            service = matched.route.service.selector().service_id(),
            method = matched.route.method.invocation_name(),
            attempt = control.attempt,
        );
        let processed = AssertUnwindSafe(
            self.execute_matched(request, &matched, &control),
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
                    ErrorCategory::DeadlineExceeded => MetricOutcome::Timeout,
                    ErrorCategory::Cancelled => MetricOutcome::Cancelled,
                    ErrorCategory::ResourceExhausted => MetricOutcome::Rejected,
                    _ => MetricOutcome::Error,
                };
                let error_code = error.code().as_str().to_owned();
                let response = encode_problem(
                    &error,
                    &control.request_id,
                    Some(path),
                    *controls_negotiated,
                );
                (outcome, Some(error_code), response)
            }
        };
        self.metrics.record(&MetricEvent::InvocationFinished(
            InvocationFinishedEvent::new(
                MetricSide::Server,
                HTTP_JSON_V1,
                Some(http_version_name(http_version)),
                matched.route.service.selector().service_id(),
                matched.route.method.invocation_name(),
                outcome,
                Some(status_class(response.status())),
                error_code.as_deref(),
                started.elapsed(),
                control.attempt,
            ),
        ));
        let mut response = response;
        if *controls_negotiated {
            response.headers_mut().insert(
                wire::REQUEST_ID,
                http::HeaderValue::from_str(&control.request_id)
                    .expect("validated request ID is a valid header value"),
            );
        } else {
            response.headers_mut().remove(wire::REQUEST_ID);
        }
        Ok(response)
    }

    async fn execute_matched(
        &self,
        request: Request<Incoming>,
        matched: &MatchedRoute,
        control: &RequestControl,
    ) -> Result<HttpResponse<GuardedBody>, Error> {
        let request_headers = application_headers(request.headers());
        let content_length = parse_content_length(request.headers())?;
        let body_required = matched.has_body();
        validate_content_type(
            request.headers(),
            matched.route.method.http_operation().consumes(),
            body_required,
        )?;
        if !body_required
            && (content_length.is_some_and(|length| length > 0)
                || request.headers().contains_key(TRANSFER_ENCODING))
        {
            return Err(unexpected_body());
        }
        let context = Context::new(ContextParts {
            side: Side::Server,
            stage: InterceptionStage::ServerHead,
            request_id: control.request_id.clone(),
            binding_id: HttpBindingId::default(),
            http_version: Some(request.version()),
            interface: matched.route.service,
            method: matched.route.method,
            deadline: control.deadline,
            attempt: std::num::NonZeroU8::new(control.attempt),
            endpoint: None,
            headers: request_headers,
            extensions: http::Extensions::new(),
            arguments: None,
            response_limit: self.max_response_body,
            response_wire_overhead: 0,
            response_budget: self.response_budget.clone(),
        });
        let terminal = HeadTerminal {
            app: self,
            request: Mutex::new(Some(request)),
            matched,
            control,
            content_length,
            body_required,
        };
        let response = control
            .deadline
            .run(Next::new(&matched.route.head_interceptor, &terminal).run(context))
            .await
            .map_err(|_| deadline_exceeded())??;
        encode_success(
            response,
            matched.route.method.http_operation().produces(),
            *matched.route.method.http_operation().method() == http::Method::HEAD,
            self.max_response_body,
            &self.response_budget,
        )
    }

    async fn execute_body(
        &self,
        request: Request<Incoming>,
        mut context: Context,
        execution: BodyExecution<'_>,
    ) -> crate::InterceptorResult {
        let BodyExecution {
            matched,
            control,
            content_length,
            body_required,
        } = execution;
        let query = request.uri().query().map(str::to_owned);
        let (_parts, body) = request.into_parts();
        let request_headers = application_headers(&_parts.headers);
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
            let body = serde_json::from_slice::<Value>(&bytes).map_err(|_| {
                Error::framework(
                    ErrorCategory::InvalidArgument,
                    "invalid_json",
                    "HTTP request body is invalid JSON",
                )
            })?;
            let arguments = matched.http_arguments(
                query.as_deref(),
                &request_headers,
                Some(body),
                self.max_query_pairs,
            )?;
            drop(body_permit);
            arguments
        } else {
            validate_body_absent(body).await?;
            matched.http_arguments(
                query.as_deref(),
                &request_headers,
                None,
                self.max_query_pairs,
            )?
        };
        context.set_stage(InterceptionStage::ServerCall);
        context.set_arguments(arguments);
        let terminal = ServiceTerminal {
            dispatch: matched.route.dispatch.as_ref(),
            max_response_body: self.max_response_body,
            response_budget: self.response_budget.clone(),
        };
        control
            .deadline
            .run(Next::new(&matched.route.interceptor, &terminal).run(context))
            .await
            .map_err(|_| deadline_exceeded())?
    }

    fn validate_head(&self, request: &Request<Incoming>) -> Result<(), Error> {
        if request.uri().to_string().len() > self.max_uri_bytes {
            return Err(Error::framework(
                ErrorCategory::InvalidArgument,
                "uri_too_large",
                "request URI exceeds the configured limit",
            ));
        }
        if request.headers().len() > self.max_headers {
            return Err(Error::framework(
                ErrorCategory::InvalidArgument,
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
            return Err(Error::framework(
                ErrorCategory::InvalidArgument,
                "headers_too_large",
                "request headers exceed the configured byte limit",
            ));
        }
        Ok(())
    }

    async fn acquire_admission(&self, deadline: Deadline) -> Result<AdmissionGuard, Error> {
        match self.admission.try_enter() {
            Ok(guard) => return Ok(guard),
            Err(AdmissionError::Draining) => return Err(draining()),
            Err(AdmissionError::Overloaded) => {}
        }
        let Some(queue) = &self.queue_slots else {
            self.metrics.record(&MetricEvent::AdmissionRejected(
                AdmissionRejectedEvent::new(MetricSide::Server, "concurrency"),
            ));
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
            Err(_) => Err(Error::framework(
                ErrorCategory::ResourceExhausted,
                "admission_queue_timeout",
                "request did not enter admission before the queue wait limit",
            )),
        }
    }
}

impl Service<Request<Incoming>> for HttpApp {
    type Response = HttpResponse<GuardedBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, request: Request<Incoming>) -> Self::Future {
        let app = self.clone();
        let is_head = request.method() == http::Method::HEAD;
        let request_id = validated_request_id_header(request.headers()).map(str::to_owned);
        let invocation_controls =
            app.invocation_controls && has_invocation_controls(request.headers());
        Box::pin(async move {
            let mut response = match AssertUnwindSafe(app.handle(request)).catch_unwind().await {
                Ok(response) => response,
                Err(_) => {
                    tracing::error!("server HTTP service panicked outside the invocation boundary");
                    let request_id =
                        request_id.unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
                    encode_problem(&request_panicked(), &request_id, None, invocation_controls)
                }
            };
            if is_head {
                *response.body_mut() = GuardedBody::new(Bytes::new(), None);
            }
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
    fn call<'a>(&'a self, context: Context) -> crate::InterceptorFuture<'a> {
        self.dispatch.call(ServerInvocation::new(
            context,
            self.max_response_body,
            self.response_budget.clone(),
        ))
    }
}

struct HeadTerminal<'a> {
    app: &'a HttpApp,
    request: Mutex<Option<Request<Incoming>>>,
    matched: &'a MatchedRoute,
    control: &'a RequestControl,
    content_length: Option<usize>,
    body_required: bool,
}

struct BodyExecution<'a> {
    matched: &'a MatchedRoute,
    control: &'a RequestControl,
    content_length: Option<usize>,
    body_required: bool,
}

impl Terminal for HeadTerminal<'_> {
    fn call<'a>(&'a self, context: Context) -> crate::InterceptorFuture<'a> {
        let request = self
            .request
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .expect("head interceptor terminal runs at most once");
        Box::pin(async move {
            self.app
                .execute_body(
                    request,
                    context,
                    BodyExecution {
                        matched: self.matched,
                        control: self.control,
                        content_length: self.content_length,
                        body_required: self.body_required,
                    },
                )
                .await
        })
    }
}

fn has_invocation_controls(headers: &HeaderMap) -> bool {
    [
        &wire::REQUEST_ID,
        &wire::TIMEOUT_MS,
        &wire::ATTEMPT,
        &wire::SERVICE_GROUP,
        &wire::SERVICE_VERSION,
    ]
    .iter()
    .any(|name| headers.contains_key(*name))
}

fn validate_service_controls(
    headers: &HeaderMap,
    expected_group: Option<&str>,
    expected_version: Option<&str>,
) -> Result<(), Error> {
    validate_service_control(
        headers,
        &wire::SERVICE_GROUP,
        expected_group,
        "service_group_mismatch",
        "x-fusen-service-group does not match the selected service",
    )?;
    validate_service_control(
        headers,
        &wire::SERVICE_VERSION,
        expected_version,
        "service_version_mismatch",
        "x-fusen-service-version does not match the selected service",
    )
}

fn validate_service_control(
    headers: &HeaderMap,
    name: &http::HeaderName,
    expected: Option<&str>,
    code: &'static str,
    message: &'static str,
) -> Result<(), Error> {
    let mut values = headers.get_all(name).iter();
    let actual = values.next();
    if values.next().is_some() {
        return Err(Error::framework(
            ErrorCategory::InvalidArgument,
            code,
            message,
        ));
    }
    let actual = match actual {
        Some(value) => Some(
            value
                .to_str()
                .map_err(|_| Error::framework(ErrorCategory::InvalidArgument, code, message))?,
        ),
        None => None,
    };
    if actual != expected {
        return Err(Error::framework(
            ErrorCategory::InvalidArgument,
            code,
            message,
        ));
    }
    Ok(())
}

fn application_headers(headers: &HeaderMap) -> HeaderMap {
    let mut headers = headers.clone();
    for name in [
        ACCEPT,
        CONNECTION,
        CONTENT_TYPE,
        CONTENT_LENGTH,
        HOST,
        TE,
        TRAILER,
        TRANSFER_ENCODING,
        UPGRADE,
        wire::REQUEST_ID,
        wire::TIMEOUT_MS,
        wire::ATTEMPT,
        wire::SERVICE_GROUP,
        wire::SERVICE_VERSION,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
    headers
}

fn overloaded() -> Error {
    Error::framework(
        ErrorCategory::ResourceExhausted,
        "overloaded",
        "server request concurrency is exhausted",
    )
}

fn draining() -> Error {
    Error::framework(ErrorCategory::Unavailable, "draining", "server is draining")
        .with_retry_hint(RetryHint::Retryable)
}

fn deadline_exceeded() -> Error {
    Error::framework(
        ErrorCategory::DeadlineExceeded,
        "deadline_exceeded",
        "service invocation deadline elapsed",
    )
}

async fn validate_body_absent(mut body: Incoming) -> Result<(), Error> {
    while let Some(frame) =
        std::future::poll_fn(|context| Pin::new(&mut body).poll_frame(context)).await
    {
        let frame = frame.map_err(|error| {
            Error::internal("HTTP body stream failed", error).with_retry_hint(RetryHint::Retryable)
        })?;
        let Ok(chunk) = frame.into_data() else {
            continue;
        };
        if !chunk.is_empty() {
            return Err(unexpected_body());
        }
    }
    Ok(())
}

fn unexpected_body() -> Error {
    Error::framework(
        ErrorCategory::InvalidArgument,
        "unexpected_body",
        "this HTTP route does not accept a request body",
    )
}

fn request_panicked() -> Error {
    Error::framework(
        ErrorCategory::Internal,
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

fn http_version_name(version: http::Version) -> &'static str {
    if version == http::Version::HTTP_2 {
        "2"
    } else {
        "1.1"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    #[test]
    fn application_headers_exclude_binding_runtime_and_hop_headers() {
        let mut headers = HeaderMap::new();
        for name in [
            "accept",
            "connection",
            "content-length",
            "content-type",
            "host",
            "keep-alive",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
            "x-request-id",
            "x-fusen-timeout-ms",
            "x-fusen-attempt",
            "x-fusen-service-group",
            "x-fusen-service-version",
        ] {
            headers.insert(name, HeaderValue::from_static("reserved"));
        }
        headers.append("x-user-header", HeaderValue::from_static("one"));
        headers.append("x-user-header", HeaderValue::from_static("two"));

        let application = application_headers(&headers);

        assert_eq!(application.len(), 2);
        assert_eq!(
            application
                .get_all("x-user-header")
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
    }
}
