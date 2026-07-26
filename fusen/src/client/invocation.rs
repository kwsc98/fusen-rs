use super::{
    endpoint_breakers::EndpointBreakerSource,
    runtime::{CLIENT_RUNNING, ClientRuntimeInner},
    transport::{HttpTransport, TransportFailureKind, circuit_open},
};
use crate::{
    Arguments, InstanceSnapshot, LoadBalancer, Router, RpcCategory, RpcContext, RpcError,
    RpcOrigin, RpcResponse,
    context::RpcContextParts,
    middleware::{MiddlewareDyn, Next, RpcResult, Terminal},
    resilience::{
        FailureClass,
        breaker::{BreakerPermit, BreakerRejection},
        retry::{RetryDecision, RetryDecisionContext, decide_with_guards, full_jitter_backoff},
    },
    runtime::{
        BoxFuture,
        admission::{AdmissionError, AdmissionGuard},
        deadline::Deadline,
    },
    wire::{decode_http_response, encode_request_template},
};
use fusen_contract::{
    InstanceId, MethodId, ServiceDescriptor, ServiceEndpoint, ServiceInstance, ServiceWeight,
    WireProtocol,
};
use fusen_observability::{MetricEvent, MetricOutcome, MetricSide};
use fusen_register::directory::{Directory, DirectoryState};
use http::header::RETRY_AFTER;
use serde::de::DeserializeOwned;
#[cfg(test)]
use serde_json::Value;
use std::{
    collections::HashSet,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant as StdInstant, SystemTime},
};
use tracing::Instrument;

pub(crate) enum EndpointSource {
    Direct(ServiceEndpoint),
    Discovery(Directory),
}

/// Erased generated client used by macro-generated `*Client` wrappers.
#[derive(Clone)]
pub struct ServiceClient {
    pub(crate) inner: Arc<ServiceClientInner>,
}

pub(crate) struct ServiceClientInner {
    pub runtime: Arc<ClientRuntimeInner>,
    pub service: &'static ServiceDescriptor,
    pub protocol: WireProtocol,
    pub source: EndpointSource,
    pub middleware: Arc<[Arc<dyn MiddlewareDyn>]>,
    pub routers: Arc<[Arc<dyn Router>]>,
    pub load_balancer: Arc<dyn LoadBalancer>,
}

impl ServiceClient {
    /// Executes one logical invocation and decodes its result within the logical deadline.
    pub async fn invoke<T, F>(&self, method_id: MethodId, build_arguments: F) -> Result<T, RpcError>
    where
        T: DeserializeOwned,
        F: FnOnce() -> Result<Arguments, RpcError> + Send,
    {
        let method = self
            .inner
            .service
            .method(method_id)
            .ok_or_else(|| crate::service::method_not_found(method_id))?;
        if self.inner.runtime.state.load(Ordering::Acquire) != CLIENT_RUNNING {
            return Err(closed_rpc());
        }
        let deadline = Deadline::after(self.inner.runtime.config.request_timeout());
        let _admission = acquire_admission(&self.inner.runtime, deadline).await?;
        let request_id = uuid::Uuid::new_v4().simple().to_string();
        let started = StdInstant::now();
        self.inner
            .runtime
            .metrics
            .record(&MetricEvent::InvocationStarted {
                side: MetricSide::Client,
                protocol: self.inner.protocol.as_str(),
                service: self.inner.service.selector().service_id(),
                method: method.fusen_identity(),
            });
        let span = tracing::info_span!(
            "fusen.client.invocation",
            request_id = %request_id,
            protocol = self.inner.protocol.as_str(),
            service = self.inner.service.selector().service_id(),
            method = method.fusen_identity(),
        );
        let invocation = async move {
            let arguments = match catch_unwind(AssertUnwindSafe(build_arguments)) {
                Ok(result) => result?,
                Err(_) => {
                    tracing::error!("RPC argument serialization panicked");
                    return Err(RpcError::framework(
                        RpcCategory::Internal,
                        "serialization_panic",
                        "failed to serialize RPC arguments",
                    ));
                }
            };
            let transport = self.inner.runtime.transport().map_err(|_| closed_rpc())?;
            let context = RpcContext::new(RpcContextParts {
                request_id: request_id.clone(),
                protocol: self.inner.protocol,
                service: self.inner.service,
                method,
                deadline,
                attempt: 1,
                headers: http::HeaderMap::new(),
                arguments,
                response_limit: self.inner.runtime.config.admission().response_body_limit(),
                response_wire_overhead: 0,
                response_budget: self.inner.runtime.response_budget.clone(),
            });
            let terminal = InvocationTerminal {
                client: self.inner.as_ref(),
                transport,
                endpoint_breaker_permit: Mutex::new(None),
            };
            let mut response = match Next::new(&self.inner.middleware, &terminal)
                .run(context)
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    terminal.succeed_endpoint_breaker();
                    return Err(error);
                }
            };
            let status = response.status();
            let attempts = response.attempts();
            let service_permit = response.take_service_breaker();
            let endpoint_permit = if response.tracks_endpoint_breaker() {
                terminal.take_endpoint_breaker()
            } else {
                terminal.succeed_endpoint_breaker();
                None
            };
            let decoded = catch_unwind(AssertUnwindSafe(|| {
                serde_json::from_slice(response.result_bytes())
            }));
            let value = match decoded {
                Ok(Ok(value)) => {
                    if let Some(permit) = endpoint_permit {
                        permit.succeed();
                    }
                    if let Some(permit) = service_permit {
                        permit.succeed();
                    }
                    value
                }
                Ok(Err(error)) => {
                    if let Some(permit) = endpoint_permit {
                        permit.fail(FailureClass::Protocol);
                    }
                    if let Some(permit) = service_permit {
                        permit.fail(FailureClass::Protocol);
                    }
                    return Err(RpcError::invalid_result(
                        "RPC result does not match the generated return type",
                        error,
                    )
                    .with_attempts(attempts));
                }
                Err(_) => {
                    if let Some(permit) = endpoint_permit {
                        permit.fail(FailureClass::Protocol);
                    }
                    if let Some(permit) = service_permit {
                        permit.fail(FailureClass::Protocol);
                    }
                    tracing::error!("RPC result deserialization panicked");
                    return Err(RpcError::framework(
                        RpcCategory::DataLoss,
                        "invalid_result",
                        "RPC result does not match the generated return type",
                    )
                    .with_attempts(attempts));
                }
            };
            Ok((value, status, attempts))
        }
        .instrument(span);
        let result = tokio::select! {
            biased;
            () = self.inner.runtime.force_cancel.cancelled() => Err(cancelled()),
            result = deadline.run(invocation) => match result {
                Ok(result) => result,
                Err(_) => Err(deadline_exceeded()),
            }
        };
        let (outcome, attempts, status_class, error_code) = match &result {
            Ok((_, status, attempts)) => (
                MetricOutcome::Success,
                *attempts,
                Some(status_class(*status)),
                None,
            ),
            Err(error) => (
                match error.category() {
                    RpcCategory::DeadlineExceeded => MetricOutcome::Timeout,
                    RpcCategory::Cancelled => MetricOutcome::Cancelled,
                    RpcCategory::ResourceExhausted => MetricOutcome::Rejected,
                    _ => MetricOutcome::Error,
                },
                error.attempts(),
                Some(status_class(error.status())),
                Some(error.code().as_str()),
            ),
        };
        self.inner
            .runtime
            .metrics
            .record(&MetricEvent::InvocationFinished {
                side: MetricSide::Client,
                protocol: self.inner.protocol.as_str(),
                service: self.inner.service.selector().service_id(),
                method: method.fusen_identity(),
                outcome,
                status_class,
                error_code,
                duration: started.elapsed(),
                attempts,
            });
        result.map(|(value, _, _)| value)
    }
}

struct InvocationTerminal<'a> {
    client: &'a ServiceClientInner,
    transport: HttpTransport,
    endpoint_breaker_permit: Mutex<Option<BreakerPermit>>,
}

impl Terminal for InvocationTerminal<'_> {
    fn call<'a>(&'a self, context: RpcContext) -> BoxFuture<'a, RpcResult> {
        Box::pin(async move { self.execute(context).await })
    }
}

impl InvocationTerminal<'_> {
    fn hold_endpoint_breaker(&self, permit: BreakerPermit) {
        let previous = self
            .endpoint_breaker_permit
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(permit);
        debug_assert!(previous.is_none(), "terminal executes at most once");
    }

    fn take_endpoint_breaker(&self) -> Option<BreakerPermit> {
        self.endpoint_breaker_permit
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }

    fn succeed_endpoint_breaker(&self) {
        if let Some(permit) = self.take_endpoint_breaker() {
            permit.succeed();
        }
    }

    async fn execute(&self, context: RpcContext) -> RpcResult {
        let template = encode_request_template(
            self.client.service,
            context.method(),
            self.client.protocol,
            context.arguments(),
            context.headers(),
            self.client.runtime.config.admission().request_body_limit(),
            &self.client.runtime.request_budget,
        )?;
        let service_breaker = self.client.runtime.service_breaker(self.client.service);
        let service_permit = service_breaker.try_acquire().map_err(|_| circuit_open())?;
        match self.execute_attempts(context, template).await {
            Ok(AttemptSuccess {
                mut response,
                endpoint_breaker_permit,
            }) => {
                self.hold_endpoint_breaker(endpoint_breaker_permit);
                response.track_endpoint_breaker();
                response.hold_service_breaker(service_permit);
                Ok(response)
            }
            Err(error) => {
                service_permit.fail(classify_rpc(&error, self.client.protocol));
                Err(error)
            }
        }
    }

    async fn execute_attempts(
        &self,
        context: RpcContext,
        template: crate::wire::RequestTemplate,
    ) -> Result<AttemptSuccess, RpcError> {
        let mut attempted_endpoints = HashSet::new();
        let mut attempt = 1u8;
        let spring_head = template.method == http::Method::HEAD;
        loop {
            let started = StdInstant::now();
            let mut attempt_context = context.clone();
            attempt_context.set_attempt(attempt);
            let selected = self.select_endpoint(&attempt_context, &attempted_endpoints)?;
            let endpoint_key = selected.instance.endpoint().as_str().to_owned();
            let bulkhead = self.client.runtime.endpoint_bulkhead(&endpoint_key);
            let bulkhead_permit = match bulkhead.try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => {
                    drop(selected.breaker_permit);
                    return Err(RpcError::framework(
                        RpcCategory::ResourceExhausted,
                        "endpoint_overloaded",
                        "selected endpoint concurrency is exhausted",
                    )
                    .with_attempts(attempt));
                }
            };
            let request = match template.to_request(
                selected.instance.endpoint(),
                context.request_id(),
                context.deadline().remaining(),
                attempt,
            ) {
                Ok(request) => request,
                Err(error) => {
                    drop(selected.breaker_permit);
                    drop(bulkhead_permit);
                    return Err(error.with_attempts(attempt));
                }
            };
            let attempt_span = tracing::info_span!(
                "fusen.client.attempt",
                request_id = %context.request_id(),
                protocol = self.client.protocol.as_str(),
                service = self.client.service.selector().service_id(),
                method = context.method().fusen_identity(),
                attempt,
                endpoint = %endpoint_key,
            );
            attempted_endpoints.insert(endpoint_key);
            let sent = tokio::select! {
                biased;
                () = self.client.runtime.force_cancel.cancelled() => {
                    drop(selected.breaker_permit);
                    drop(bulkhead_permit);
                    return Err(cancelled().with_attempts(attempt));
                },
                result = context
                    .deadline()
                    .run(self.transport.send(request).instrument(attempt_span.clone())) => result,
            };
            let (result, failure, retry_after): (
                Result<RpcResponse, RpcError>,
                FailureClass,
                Option<Duration>,
            ) = match sent {
                Err(_) => {
                    let failure = FailureClass::Timeout;
                    selected.breaker_permit.fail(failure);
                    (Err(deadline_exceeded()), failure, None)
                }
                Ok(Err(error)) => {
                    let failure = match error.kind {
                        TransportFailureKind::Connect => FailureClass::Connect,
                        TransportFailureKind::Io => FailureClass::Transport,
                    };
                    selected.breaker_permit.fail(failure);
                    (Err(error.into_rpc()), failure, None)
                }
                Ok(Ok(response)) => {
                    let retry_after = parse_retry_after(response.headers());
                    match context
                        .deadline()
                        .run(
                            decode_http_response(
                                self.client.protocol,
                                spring_head,
                                response,
                                self.client.runtime.config.admission().response_body_limit(),
                                &self.client.runtime.response_budget,
                            )
                            .instrument(attempt_span),
                        )
                        .await
                    {
                        Err(_) => {
                            let failure = FailureClass::Timeout;
                            selected.breaker_permit.fail(failure);
                            (Err(deadline_exceeded()), failure, retry_after)
                        }
                        Ok(Ok(response)) => {
                            drop(bulkhead_permit);
                            self.client
                                .runtime
                                .metrics
                                .record(&MetricEvent::AttemptFinished {
                                    protocol: self.client.protocol.as_str(),
                                    service: self.client.service.selector().service_id(),
                                    method: context.method().fusen_identity(),
                                    attempt,
                                    outcome: MetricOutcome::Success,
                                    failure_class: None,
                                    duration: started.elapsed(),
                                });
                            let mut response = response;
                            response.set_attempts(attempt);
                            return Ok(AttemptSuccess {
                                response,
                                endpoint_breaker_permit: selected.breaker_permit,
                            });
                        }
                        Ok(Err(error)) => {
                            let failure = classify_rpc(&error, self.client.protocol);
                            selected.breaker_permit.fail(failure);
                            (Err(error), failure, retry_after)
                        }
                    }
                }
            };
            drop(bulkhead_permit);
            self.client
                .runtime
                .metrics
                .record(&MetricEvent::AttemptFinished {
                    protocol: self.client.protocol.as_str(),
                    service: self.client.service.selector().service_id(),
                    method: context.method().fusen_identity(),
                    attempt,
                    outcome: if failure == FailureClass::Timeout {
                        MetricOutcome::Timeout
                    } else {
                        MetricOutcome::Error
                    },
                    failure_class: Some(failure_name(failure)),
                    duration: started.elapsed(),
                });
            let error = result
                .expect_err("failed attempt contains an RPC error")
                .with_attempts(attempt);
            let (base, cap) = (
                self.client.runtime.config.retry().backoff_base_value(),
                self.client.runtime.config.retry().backoff_cap_value(),
            );
            let mut delay = {
                let mut rng = rand::rng();
                full_jitter_backoff(base, cap, attempt, &mut rng)
            };
            if let Some(retry_after) = retry_after {
                delay = delay.max(retry_after);
            }
            let remaining = context.deadline().remaining();
            if delay >= remaining {
                return Err(error);
            }
            let decision = RetryDecisionContext::new(
                attempt,
                self.client.runtime.config.retry().max_attempts_value(),
                context.method().idempotency(),
                failure,
                remaining,
            );
            let budget = self.client.runtime.retry_budget(self.client.service);
            let policy_decision = catch_unwind(AssertUnwindSafe(|| {
                decide_with_guards(
                    self.client.runtime.retry_policy.as_ref(),
                    &decision,
                    &budget,
                )
            }));
            if !matches!(policy_decision, Ok(RetryDecision::Retry)) {
                if policy_decision.is_err() {
                    tracing::error!("retry policy panicked and was isolated");
                    return Err(RpcError::framework(
                        RpcCategory::Internal,
                        "retry_policy_panic",
                        "retry policy failed",
                    )
                    .with_attempts(attempt));
                }
                return Err(error);
            }
            tokio::select! {
                biased;
                () = self.client.runtime.force_cancel.cancelled() => {
                    return Err(cancelled().with_attempts(attempt));
                }
                () = tokio::time::sleep(delay) => {}
            }
            attempt = attempt.saturating_add(1);
        }
    }

    fn select_endpoint(
        &self,
        context: &RpcContext,
        attempted: &HashSet<String>,
    ) -> Result<SelectedEndpoint, RpcError> {
        let (mut instances, source) = match &self.client.source {
            EndpointSource::Direct(endpoint) => (
                vec![ServiceInstance::new(
                    InstanceId::new("direct").expect("static direct instance ID is valid"),
                    endpoint.clone(),
                    ServiceWeight::default(),
                )],
                EndpointBreakerSource::Direct,
            ),
            EndpointSource::Discovery(directory) => {
                let snapshot = directory.snapshot();
                if !matches!(
                    snapshot.state(),
                    DirectoryState::Ready | DirectoryState::Stale
                ) {
                    return Err(no_instances());
                }
                (
                    snapshot.instances().to_vec(),
                    EndpointBreakerSource::Discovery,
                )
            }
        };
        let has_untried = instances
            .iter()
            .any(|instance| !attempted.contains(instance.endpoint().as_str()));
        if has_untried {
            instances.retain(|instance| !attempted.contains(instance.endpoint().as_str()));
        }
        let mut routed = InstanceSnapshot::new(instances);
        for router in self.client.routers.iter() {
            routed = match catch_unwind(AssertUnwindSafe(|| router.route(context, routed.clone())))
            {
                Ok(Ok(instances)) => instances,
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    tracing::error!("router panicked and was isolated");
                    return Err(RpcError::framework(
                        RpcCategory::Internal,
                        "router_panic",
                        "router failed",
                    ));
                }
            };
        }
        let mut eligible = Vec::new();
        let mut permits = Vec::new();
        for instance in routed.iter() {
            let breaker = self.client.runtime.endpoint_breaker(
                self.client.service,
                self.client.protocol,
                source,
                instance.endpoint().as_str(),
            );
            match breaker.try_acquire() {
                Ok(permit) => {
                    eligible.push(instance.clone());
                    permits.push(UnattemptedBreakerPermit::new(permit));
                }
                Err(BreakerRejection::Open { .. } | BreakerRejection::HalfOpenSaturated) => {}
            }
        }
        if eligible.is_empty() {
            return Err(circuit_open());
        }
        let eligible = InstanceSnapshot::new(eligible);
        let index = match catch_unwind(AssertUnwindSafe(|| {
            self.client.load_balancer.select(context, &eligible)
        })) {
            Ok(Ok(index)) if index < eligible.len() => index,
            Ok(Ok(_)) => {
                return Err(RpcError::framework(
                    RpcCategory::Internal,
                    "invalid_load_balancer_selection",
                    "load balancer returned an out-of-range index",
                ));
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                tracing::error!("load balancer panicked and was isolated");
                return Err(RpcError::framework(
                    RpcCategory::Internal,
                    "load_balancer_panic",
                    "load balancer failed",
                ));
            }
        };
        let breaker_permit = permits[index]
            .take()
            .expect("eligible endpoint has a breaker permit");
        Ok(SelectedEndpoint {
            instance: eligible[index].clone(),
            breaker_permit,
        })
    }
}

struct AttemptSuccess {
    response: RpcResponse,
    endpoint_breaker_permit: BreakerPermit,
}

struct UnattemptedBreakerPermit(Option<BreakerPermit>);

impl UnattemptedBreakerPermit {
    fn new(permit: BreakerPermit) -> Self {
        Self(Some(permit))
    }

    fn take(&mut self) -> Option<BreakerPermit> {
        self.0.take()
    }
}

impl Drop for UnattemptedBreakerPermit {
    fn drop(&mut self) {
        if let Some(permit) = self.0.take() {
            permit.release_unattempted();
        }
    }
}

struct SelectedEndpoint {
    instance: ServiceInstance,
    breaker_permit: BreakerPermit,
}

async fn acquire_admission(
    runtime: &Arc<ClientRuntimeInner>,
    deadline: Deadline,
) -> Result<AdmissionGuard, RpcError> {
    match runtime.admission.try_enter() {
        Ok(guard) => return Ok(guard),
        Err(AdmissionError::Draining) => return Err(closed_rpc()),
        Err(AdmissionError::Overloaded) => {}
    }
    let Some(queue) = &runtime.queue_slots else {
        runtime.metrics.record(&MetricEvent::AdmissionRejected {
            side: MetricSide::Client,
            reason: "concurrency",
        });
        return Err(RpcError::framework(
            RpcCategory::ResourceExhausted,
            "overloaded",
            "client logical invocation concurrency is exhausted",
        ));
    };
    let queue_permit = queue.clone().try_acquire_owned().map_err(|_| {
        RpcError::framework(
            RpcCategory::ResourceExhausted,
            "admission_queue_full",
            "client admission queue is full",
        )
    })?;
    let queue_deadline = deadline.min(Deadline::after(
        runtime.config.admission().queue_value().max_wait(),
    ));
    let result = queue_deadline.run(runtime.admission.enter()).await;
    drop(queue_permit);
    match result {
        Ok(Ok(guard)) => Ok(guard),
        Ok(Err(_)) => Err(closed_rpc()),
        Err(_) if deadline.is_elapsed() => Err(deadline_exceeded()),
        Err(_) => Err(RpcError::framework(
            RpcCategory::ResourceExhausted,
            "admission_queue_timeout",
            "client admission queue wait elapsed",
        )),
    }
}

fn classify_rpc(error: &RpcError, protocol: WireProtocol) -> FailureClass {
    if error.origin() == RpcOrigin::Application {
        return FailureClass::Application;
    }
    if error.origin() == RpcOrigin::Local {
        if error.retryable() {
            return FailureClass::Transport;
        }
        return match error.category() {
            RpcCategory::DeadlineExceeded => FailureClass::Timeout,
            RpcCategory::Cancelled => FailureClass::Cancelled,
            RpcCategory::DataLoss => FailureClass::Protocol,
            RpcCategory::ResourceExhausted | RpcCategory::Unavailable => {
                FailureClass::LocalRejection
            }
            _ => FailureClass::InvalidRequest,
        };
    }
    let retryable_status = matches!(
        error.status().as_u16(),
        408 | 425 | 429 | 500 | 502 | 503 | 504
    );
    if retryable_status {
        let failure = match error.status().as_u16() {
            408 => FailureClass::Timeout,
            429 => FailureClass::Overloaded,
            502..=504 => FailureClass::Unavailable,
            _ => FailureClass::RemoteServer,
        };
        return match protocol {
            WireProtocol::FusenV1 if !error.retryable() => FailureClass::RemoteFailure,
            WireProtocol::FusenV1 | WireProtocol::SpringCloudV1 => failure,
            _ => FailureClass::RemoteFailure,
        };
    }
    if error.category() == RpcCategory::DataLoss {
        FailureClass::Protocol
    } else {
        FailureClass::Application
    }
}

fn parse_retry_after(headers: &http::HeaderMap) -> Option<Duration> {
    parse_retry_after_at(headers, SystemTime::now())
}

fn parse_retry_after_at(headers: &http::HeaderMap, now: SystemTime) -> Option<Duration> {
    let mut values = headers.get_all(RETRY_AFTER).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let value = value.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(now).unwrap_or(Duration::ZERO))
}

fn failure_name(failure: FailureClass) -> &'static str {
    match failure {
        FailureClass::Connect => "connect",
        FailureClass::Transport => "transport",
        FailureClass::Timeout => "timeout",
        FailureClass::Unavailable => "unavailable",
        FailureClass::Overloaded => "overloaded",
        FailureClass::RemoteServer => "remote_server",
        FailureClass::RemoteFailure => "remote_failure",
        FailureClass::Protocol => "protocol",
        FailureClass::Application => "application",
        FailureClass::InvalidRequest => "invalid_request",
        FailureClass::Cancelled => "cancelled",
        FailureClass::LocalRejection => "local_rejection",
    }
}

fn no_instances() -> RpcError {
    RpcError::framework(
        RpcCategory::Unavailable,
        "no_instances",
        "discovery has no currently routable service instances",
    )
}

fn closed_rpc() -> RpcError {
    RpcError::framework(
        RpcCategory::Unavailable,
        "client_closed",
        "client runtime is draining or closed",
    )
}

fn cancelled() -> RpcError {
    RpcError::framework(
        RpcCategory::Cancelled,
        "cancelled",
        "RPC invocation was cancelled",
    )
}

fn deadline_exceeded() -> RpcError {
    RpcError::framework(
        RpcCategory::DeadlineExceeded,
        "deadline_exceeded",
        "RPC deadline elapsed",
    )
}

fn status_class(status: http::StatusCode) -> &'static str {
    match status.as_u16() / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        _ => "5xx",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BreakerThreshold, CircuitBreakerConfig, ClientConfig, ClientRuntime, ErrorCode,
        InstanceSnapshot, LoadBalancer, ProblemDetails, RetryConfig, Router,
        middleware::{MiddlewareDyn, erase_middleware},
        resilience::breaker::BreakerState,
        runtime::budget::ByteBudget,
        wire::{
            ATTEMPT, JSON_CONTENT_TYPE, PROBLEM_CONTENT_TYPE, REQUEST_ID, SERVICE_GROUP, TIMEOUT_MS,
        },
    };
    use bytes::Bytes;
    use fusen_contract::{
        Idempotency, MethodDescriptor, MethodId, ServiceInstance, ServiceSelector,
        SpringCloudMethod,
    };
    use fusen_observability::MetricsRecorder;
    use fusen_register::directory::{DirectoryPublisher, directory};
    use futures_util::stream;
    use http::{
        HeaderMap, HeaderValue, Method, Request, Response, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE},
    };
    use http_body_util::{BodyExt, Full, StreamBody};
    use hyper::{
        body::{Frame, Incoming},
        service::service_fn,
    };
    use hyper_util::rt::TokioIo;
    use serde::Deserialize;
    use serde_json::json;
    use std::{
        convert::Infallible,
        io,
        sync::{
            Arc, OnceLock,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};

    #[derive(Debug)]
    struct CapturedAttempt {
        endpoint: &'static str,
        request_id: String,
        attempt: u8,
    }

    #[derive(Clone, Default)]
    struct InvocationMetrics {
        started: Arc<AtomicUsize>,
        succeeded: Arc<AtomicUsize>,
        failed: Arc<AtomicUsize>,
    }

    impl MetricsRecorder for InvocationMetrics {
        fn record(&self, event: &MetricEvent<'_>) {
            match event {
                MetricEvent::InvocationStarted {
                    side: MetricSide::Client,
                    ..
                } => {
                    self.started.fetch_add(1, Ordering::SeqCst);
                }
                MetricEvent::InvocationFinished {
                    side: MetricSide::Client,
                    outcome: MetricOutcome::Success,
                    ..
                } => {
                    self.succeeded.fetch_add(1, Ordering::SeqCst);
                }
                MetricEvent::InvocationFinished {
                    side: MetricSide::Client,
                    ..
                } => {
                    self.failed.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    }

    #[derive(Debug, Deserialize)]
    struct StructuredResult {
        _value: String,
    }

    #[derive(Clone, Copy)]
    struct FirstEndpoint;

    impl LoadBalancer for FirstEndpoint {
        fn select(
            &self,
            _context: &RpcContext,
            instances: &InstanceSnapshot,
        ) -> Result<usize, RpcError> {
            assert!(
                !instances.is_empty(),
                "fixture always publishes an endpoint"
            );
            Ok(0)
        }
    }

    struct PanickingRouter;

    impl Router for PanickingRouter {
        fn route(
            &self,
            _context: &RpcContext,
            _instances: InstanceSnapshot,
        ) -> Result<InstanceSnapshot, RpcError> {
            panic!("private router panic")
        }
    }

    struct PanickingLoadBalancer;

    impl LoadBalancer for PanickingLoadBalancer {
        fn select(
            &self,
            _context: &RpcContext,
            _instances: &InstanceSnapshot,
        ) -> Result<usize, RpcError> {
            panic!("private load balancer panic")
        }
    }

    struct PanickingRetryPolicy;

    impl crate::RetryPolicy for PanickingRetryPolicy {
        fn decide(&self, _context: &RetryDecisionContext) -> RetryDecision {
            panic!("private retry policy panic")
        }
    }

    struct ReplaceRemoteResult;

    impl crate::Middleware for ReplaceRemoteResult {
        async fn handle<'a>(&'a self, context: RpcContext, next: Next<'a>) -> RpcResult {
            let local_response = context.clone();
            drop(next.run(context).await?);
            local_response.respond("middleware-result")
        }
    }

    fn remote_error(status: StatusCode, retryable: bool) -> RpcError {
        RpcError::from_remote(ProblemDetails {
            type_uri: "urn:fusen:error:remote_test".to_owned(),
            title: "remote test".to_owned(),
            status: status.as_u16(),
            detail: None,
            instance: None,
            code: ErrorCode::new("remote_test").unwrap(),
            request_id: "request-1".to_owned(),
            retryable,
        })
    }

    fn replay_service() -> &'static ServiceDescriptor {
        Box::leak(Box::new(
            ServiceDescriptor::new(
                ServiceSelector::new("replay", None, None).unwrap(),
                vec![
                    MethodDescriptor::new(MethodId::new(0), "call", Idempotency::Idempotent, None)
                        .unwrap(),
                ],
            )
            .unwrap(),
        ))
    }

    fn resilience_service() -> &'static ServiceDescriptor {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        SERVICE.get_or_init(|| {
            ServiceDescriptor::new(
                ServiceSelector::new("resilience", None, None).unwrap(),
                vec![
                    MethodDescriptor::new(
                        MethodId::new(0),
                        "call",
                        Idempotency::Safe,
                        Some(SpringCloudMethod::new(Method::GET, "/call", Vec::new()).unwrap()),
                    )
                    .unwrap(),
                ],
            )
            .unwrap()
        })
    }

    fn resilience_config(request_timeout: Duration, retry: RetryConfig) -> ClientConfig {
        resilience_config_with_endpoint_close_successes(request_timeout, retry, 1)
    }

    fn resilience_config_with_endpoint_close_successes(
        request_timeout: Duration,
        retry: RetryConfig,
        endpoint_close_successes: u32,
    ) -> ClientConfig {
        let endpoint = BreakerThreshold::endpoint_defaults().thresholds(
            Duration::from_secs(10),
            10,
            1,
            1.0,
            Duration::from_secs(10),
            1,
            endpoint_close_successes,
        );
        let service = BreakerThreshold::service_defaults().thresholds(
            Duration::from_secs(30),
            10,
            1,
            1.0,
            Duration::from_secs(15),
            1,
            1,
        );
        ClientConfig::builder()
            .request_timeout(request_timeout)
            .retry(retry)
            .circuit_breaker(
                CircuitBreakerConfig::default()
                    .endpoint(endpoint)
                    .service(service),
            )
            .build()
            .unwrap()
    }

    fn discovered_client(
        runtime: &ClientRuntime,
        instances: Vec<ServiceInstance>,
    ) -> (DirectoryPublisher, ServiceClient) {
        let (publisher, directory) = directory();
        runtime.inner.endpoint_breakers.replace_discovery(
            resilience_service().selector(),
            WireProtocol::SpringCloudV1,
            &instances,
        );
        publisher.publish_ready(instances).unwrap();
        let client = ServiceClient {
            inner: Arc::new(ServiceClientInner {
                runtime: runtime.inner.clone(),
                service: resilience_service(),
                protocol: WireProtocol::SpringCloudV1,
                source: EndpointSource::Discovery(directory),
                middleware: Arc::from(Vec::<Arc<dyn MiddlewareDyn>>::new()),
                routers: Arc::from(Vec::<Arc<dyn Router>>::new()),
                load_balancer: Arc::new(FirstEndpoint),
            }),
        };
        (publisher, client)
    }

    fn direct_client(runtime: &ClientRuntime, endpoint: ServiceEndpoint) -> ServiceClient {
        ServiceClient {
            inner: Arc::new(ServiceClientInner {
                runtime: runtime.inner.clone(),
                service: resilience_service(),
                protocol: WireProtocol::SpringCloudV1,
                source: EndpointSource::Direct(endpoint),
                middleware: Arc::from(Vec::<Arc<dyn MiddlewareDyn>>::new()),
                routers: Arc::from(Vec::<Arc<dyn Router>>::new()),
                load_balancer: Arc::new(FirstEndpoint),
            }),
        }
    }

    fn instance(id: &str, endpoint: ServiceEndpoint) -> ServiceInstance {
        ServiceInstance::new(
            InstanceId::new(id).unwrap(),
            endpoint,
            ServiceWeight::default(),
        )
    }

    async fn capture_request(
        endpoint: &'static str,
        request: Request<Incoming>,
        captured: &mpsc::UnboundedSender<CapturedAttempt>,
    ) {
        let request_id = request
            .headers()
            .get(REQUEST_ID)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let attempt = request
            .headers()
            .get(ATTEMPT)
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        request.into_body().collect().await.unwrap();
        captured
            .send(CapturedAttempt {
                endpoint,
                request_id,
                attempt,
            })
            .unwrap();
    }

    async fn spawn_broken_body_endpoint(
        captured: mpsc::UnboundedSender<CapturedAttempt>,
    ) -> (ServiceEndpoint, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service = service_fn(move |request| {
                let captured = captured.clone();
                async move {
                    capture_request("broken", request, &captured).await;
                    let frames = stream::iter([
                        Ok::<_, io::Error>(Frame::data(Bytes::from_static(b"\""))),
                        Err(io::Error::new(
                            io::ErrorKind::ConnectionReset,
                            "controlled response reset",
                        )),
                    ]);
                    Ok::<_, Infallible>(
                        Response::builder()
                            .header(CONTENT_TYPE, JSON_CONTENT_TYPE)
                            .header(CONTENT_LENGTH, "8")
                            .body(StreamBody::new(frames))
                            .unwrap(),
                    )
                }
            });
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder.keep_alive(false);
            let _ = builder
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
        (endpoint, fixture)
    }

    async fn spawn_full_endpoint(
        endpoint_name: &'static str,
        status: StatusCode,
        body: Bytes,
        retry_after: Option<&'static str>,
        captured: mpsc::UnboundedSender<CapturedAttempt>,
    ) -> (ServiceEndpoint, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service = service_fn(move |request| {
                let captured = captured.clone();
                let body = body.clone();
                async move {
                    capture_request(endpoint_name, request, &captured).await;
                    let mut response = Response::builder().status(status).header(
                        CONTENT_TYPE,
                        if status.is_success() {
                            JSON_CONTENT_TYPE
                        } else {
                            PROBLEM_CONTENT_TYPE
                        },
                    );
                    if let Some(retry_after) = retry_after {
                        response = response.header(RETRY_AFTER, retry_after);
                    }
                    Ok::<_, Infallible>(response.body(Full::new(body)).unwrap())
                }
            });
            let mut builder = hyper::server::conn::http1::Builder::new();
            builder.keep_alive(false);
            builder
                .serve_connection(TokioIo::new(stream), service)
                .await
                .unwrap();
        });
        (endpoint, fixture)
    }

    #[tokio::test]
    async fn request_template_is_replayable_with_independent_control_headers_and_budget() {
        let service = replay_service();
        let method = service.method(MethodId::new(0)).unwrap();
        let endpoint: ServiceEndpoint = "http://127.0.0.1:8080".parse().unwrap();
        let mut arguments = Arguments::new();
        arguments.insert("value".to_owned(), json!({"nested": [1, 2, 3]}));
        let mut application_headers = HeaderMap::new();
        application_headers.insert("authorization", HeaderValue::from_static("Bearer test"));
        application_headers.insert(REQUEST_ID, HeaderValue::from_static("middleware-spoof"));
        application_headers.insert(SERVICE_GROUP, HeaderValue::from_static("middleware-spoof"));
        let budget = ByteBudget::new(1024);
        let template = encode_request_template(
            service,
            method,
            WireProtocol::FusenV1,
            &arguments,
            &application_headers,
            1024,
            &budget,
        )
        .unwrap();
        let body_len = template.body.len();
        assert_eq!(budget.used(), body_len);

        let first = template
            .to_request(&endpoint, "same-request", Duration::from_millis(1500), 1)
            .unwrap();
        let second = template
            .to_request(&endpoint, "same-request", Duration::from_millis(900), 2)
            .unwrap();
        assert_eq!(budget.used(), body_len);
        assert_eq!(first.method(), second.method());
        assert_eq!(first.uri(), second.uri());
        assert_eq!(first.headers()[REQUEST_ID], "same-request");
        assert_eq!(second.headers()[REQUEST_ID], "same-request");
        assert_eq!(first.headers()[ATTEMPT], "1");
        assert_eq!(second.headers()[ATTEMPT], "2");
        assert_eq!(first.headers()[TIMEOUT_MS], "1500");
        assert_eq!(second.headers()[TIMEOUT_MS], "900");
        assert_eq!(first.headers()["authorization"], "Bearer test");
        assert!(!first.headers().contains_key(SERVICE_GROUP));

        let first_body = first.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(budget.used(), body_len);
        let second_body = second.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(budget.used(), body_len);
        assert_eq!(first_body, template.body);
        assert_eq!(second_body, template.body);
        drop(template);
        assert_eq!(budget.used(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn endpoint_selection_releases_unselected_half_open_permit_without_resetting_successes() {
        let first_endpoint: ServiceEndpoint = "http://127.0.0.1:8081".parse().unwrap();
        let second_endpoint: ServiceEndpoint = "http://127.0.0.1:8082".parse().unwrap();
        let config = resilience_config_with_endpoint_close_successes(
            Duration::from_secs(2),
            RetryConfig::default(),
            2,
        );
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let (_publisher, client) = discovered_client(
            &runtime,
            vec![
                instance("first", first_endpoint.clone()),
                instance("second", second_endpoint.clone()),
            ],
        );
        let first_breaker = runtime.inner.endpoint_breaker(
            resilience_service(),
            WireProtocol::SpringCloudV1,
            EndpointBreakerSource::Discovery,
            first_endpoint.as_str(),
        );
        let second_breaker = runtime.inner.endpoint_breaker(
            resilience_service(),
            WireProtocol::SpringCloudV1,
            EndpointBreakerSource::Discovery,
            second_endpoint.as_str(),
        );
        first_breaker
            .try_acquire()
            .unwrap()
            .fail(FailureClass::Transport);
        second_breaker
            .try_acquire()
            .unwrap()
            .fail(FailureClass::Transport);
        tokio::time::advance(Duration::from_secs(10)).await;
        second_breaker.try_acquire().unwrap().succeed();

        {
            let terminal = InvocationTerminal {
                client: client.inner.as_ref(),
                transport: runtime.inner.transport().unwrap(),
                endpoint_breaker_permit: Mutex::new(None),
            };
            let service = resilience_service();
            let context = RpcContext::new(RpcContextParts {
                request_id: "selection-test".to_owned(),
                protocol: WireProtocol::SpringCloudV1,
                service,
                method: service.method(MethodId::new(0)).unwrap(),
                deadline: Deadline::after(Duration::from_secs(1)),
                attempt: 1,
                headers: HeaderMap::new(),
                arguments: Arguments::new(),
                response_limit: runtime.inner.config.admission().response_body_limit(),
                response_wire_overhead: 0,
                response_budget: runtime.inner.response_budget.clone(),
            });
            let selected = terminal.select_endpoint(&context, &HashSet::new()).unwrap();
            assert_eq!(selected.instance.endpoint(), &first_endpoint);
            selected.breaker_permit.release_unattempted();
        }

        second_breaker.try_acquire().unwrap().succeed();
        assert_eq!(second_breaker.snapshot().state, BreakerState::Closed);

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn router_and_load_balancer_panics_are_isolated_before_network_io() {
        let endpoint: ServiceEndpoint = "http://127.0.0.1:1".parse().unwrap();
        let runtime = ClientRuntime::builder().build().unwrap();

        let mut router_client = direct_client(&runtime, endpoint.clone());
        Arc::get_mut(&mut router_client.inner).unwrap().routers =
            Arc::from([Arc::new(PanickingRouter) as Arc<dyn Router>]);
        let error = router_client
            .invoke::<Value, _>(MethodId::new(0), || Ok(Arguments::new()))
            .await
            .unwrap_err();
        assert_eq!(error.code().as_str(), "router_panic");

        let mut load_balancer_client = direct_client(&runtime, endpoint);
        Arc::get_mut(&mut load_balancer_client.inner)
            .unwrap()
            .load_balancer = Arc::new(PanickingLoadBalancer);
        let error = load_balancer_client
            .invoke::<Value, _>(MethodId::new(0), || Ok(Arguments::new()))
            .await
            .unwrap_err();
        assert_eq!(error.code().as_str(), "load_balancer_panic");

        drop(router_client);
        drop(load_balancer_client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_policy_panic_stops_after_the_first_attempt() {
        let problem = ProblemDetails {
            type_uri: "urn:fusen:error:unavailable:fixture_unavailable".to_owned(),
            title: "Service Unavailable".to_owned(),
            status: StatusCode::SERVICE_UNAVAILABLE.as_u16(),
            detail: Some("retry later".to_owned()),
            instance: None,
            code: ErrorCode::new("fixture_unavailable").unwrap(),
            request_id: "fixture-request".to_owned(),
            retryable: true,
        };
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "retry-policy-panic",
            StatusCode::SERVICE_UNAVAILABLE,
            Bytes::from(serde_json::to_vec(&problem).unwrap()),
            None,
            captured_tx,
        )
        .await;
        let runtime = ClientRuntime::builder()
            .config(resilience_config(
                Duration::from_secs(1),
                RetryConfig::default().backoff(Duration::from_nanos(1), Duration::from_nanos(1)),
            ))
            .retry_policy(PanickingRetryPolicy)
            .build()
            .unwrap();
        let client = direct_client(&runtime, endpoint);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), || Ok(Arguments::new()))
            .await
            .unwrap_err();
        assert_eq!(error.code().as_str(), "retry_policy_panic");
        assert_eq!(error.attempts(), 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transient_body_read_retries_an_untried_endpoint_with_stable_request_identity() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (broken_endpoint, broken_fixture) =
            spawn_broken_body_endpoint(captured_tx.clone()).await;
        let (healthy_endpoint, healthy_fixture) = spawn_full_endpoint(
            "healthy",
            StatusCode::OK,
            Bytes::from_static(br#""healthy""#),
            None,
            captured_tx,
        )
        .await;
        let config = resilience_config(
            Duration::from_secs(2),
            RetryConfig::default().backoff(Duration::from_nanos(1), Duration::from_nanos(1)),
        );
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let (_publisher, client) = discovered_client(
            &runtime,
            vec![
                instance("broken", broken_endpoint.clone()),
                instance("healthy", healthy_endpoint.clone()),
            ],
        );

        let value: Value = client
            .invoke(MethodId::new(0), || Ok(Arguments::new()))
            .await
            .unwrap();
        assert_eq!(value, json!("healthy"));
        let first = captured_rx.recv().await.unwrap();
        let second = captured_rx.recv().await.unwrap();
        assert_eq!((first.endpoint, first.attempt), ("broken", 1));
        assert_eq!((second.endpoint, second.attempt), ("healthy", 2));
        assert_eq!(first.request_id, second.request_id);

        let endpoint = runtime
            .inner
            .endpoint_breaker(
                resilience_service(),
                WireProtocol::SpringCloudV1,
                EndpointBreakerSource::Discovery,
                broken_endpoint.as_str(),
            )
            .snapshot();
        assert_eq!(endpoint.state, BreakerState::Open);
        let healthy = runtime
            .inner
            .endpoint_breaker(
                resilience_service(),
                WireProtocol::SpringCloudV1,
                EndpointBreakerSource::Discovery,
                healthy_endpoint.as_str(),
            )
            .snapshot();
        assert_eq!((healthy.samples, healthy.failures), (1, 0));
        let service = runtime
            .inner
            .service_breaker(resilience_service())
            .snapshot();
        assert_eq!(service.state, BreakerState::Closed);
        assert_eq!((service.samples, service.failures), (1, 0));

        broken_fixture.await.unwrap();
        healthy_fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn infeasible_retry_after_stops_without_spending_a_retry_token() {
        let problem = ProblemDetails {
            type_uri: "urn:fusen:error:unavailable:fixture_unavailable".to_owned(),
            title: "Service Unavailable".to_owned(),
            status: StatusCode::SERVICE_UNAVAILABLE.as_u16(),
            detail: Some("retry later".to_owned()),
            instance: None,
            code: ErrorCode::new("fixture_unavailable").unwrap(),
            request_id: "fixture-request".to_owned(),
            retryable: true,
        };
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "retry-after",
            StatusCode::SERVICE_UNAVAILABLE,
            Bytes::from(serde_json::to_vec(&problem).unwrap()),
            Some("60"),
            captured_tx,
        )
        .await;
        let config = resilience_config(
            Duration::from_millis(100),
            RetryConfig::default()
                .backoff(Duration::from_nanos(1), Duration::from_nanos(1))
                .budget(1, 1),
        );
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let client = direct_client(&runtime, endpoint);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), || Ok(Arguments::new()))
            .await
            .expect_err("Retry-After does not fit in the logical deadline");
        assert_eq!(error.attempts(), 1);
        let captured = captured_rx.recv().await.unwrap();
        assert_eq!(captured.attempt, 1);
        assert_eq!(
            runtime.inner.retry_budget(resilience_service()).available(),
            1
        );

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn typed_result_decode_failure_is_the_logical_error_terminal() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "invalid-result",
            StatusCode::OK,
            Bytes::from_static(br#""not-an-object""#),
            None,
            captured_tx,
        )
        .await;
        let metrics = InvocationMetrics::default();
        let runtime = ClientRuntime::builder()
            .metrics(metrics.clone())
            .build()
            .unwrap();
        let endpoint_key = endpoint.as_str().to_owned();
        let client = direct_client(&runtime, endpoint);

        let error = client
            .invoke::<StructuredResult, _>(MethodId::new(0), || Ok(Arguments::new()))
            .await
            .expect_err("a scalar result cannot decode into the generated object type");
        assert_eq!(error.category(), RpcCategory::DataLoss);
        assert_eq!(error.code().as_str(), "invalid_result");
        assert_eq!(error.attempts(), 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert_eq!(metrics.started.load(Ordering::SeqCst), 1);
        assert_eq!(metrics.succeeded.load(Ordering::SeqCst), 0);
        assert_eq!(metrics.failed.load(Ordering::SeqCst), 1);
        let service = runtime
            .inner
            .service_breaker(resilience_service())
            .snapshot();
        assert_eq!((service.samples, service.failures), (1, 1));
        let endpoint = runtime
            .inner
            .endpoint_breaker(
                resilience_service(),
                WireProtocol::SpringCloudV1,
                EndpointBreakerSource::Direct,
                &endpoint_key,
            )
            .snapshot();
        assert_eq!((endpoint.samples, endpoint.failures), (1, 1));

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn middleware_replacement_decode_failure_does_not_poison_breakers() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "middleware-replacement",
            StatusCode::OK,
            Bytes::from_static(br#"{"_value":"remote"}"#),
            None,
            captured_tx,
        )
        .await;
        let runtime = ClientRuntime::builder()
            .config(resilience_config(
                Duration::from_secs(1),
                RetryConfig::default(),
            ))
            .build()
            .unwrap();
        let endpoint_key = endpoint.as_str().to_owned();
        let mut client = direct_client(&runtime, endpoint);
        Arc::get_mut(&mut client.inner).unwrap().middleware =
            Arc::from([erase_middleware(ReplaceRemoteResult)]);

        let error = client
            .invoke::<StructuredResult, _>(MethodId::new(0), || Ok(Arguments::new()))
            .await
            .expect_err("middleware replacement does not match the generated return type");
        assert_eq!(error.category(), RpcCategory::DataLoss);
        assert_eq!(error.code().as_str(), "invalid_result");
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);

        let endpoint = runtime
            .inner
            .endpoint_breaker(
                resilience_service(),
                WireProtocol::SpringCloudV1,
                EndpointBreakerSource::Direct,
                &endpoint_key,
            )
            .snapshot();
        assert_eq!((endpoint.samples, endpoint.failures), (1, 0));
        let service = runtime
            .inner
            .service_breaker(resilience_service())
            .snapshot();
        assert_eq!((service.samples, service.failures), (0, 0));

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[test]
    fn retry_after_accepts_delta_seconds_and_http_date() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("12"));
        assert_eq!(
            parse_retry_after_at(&headers, now),
            Some(Duration::from_secs(12))
        );

        headers.append(RETRY_AFTER, HeaderValue::from_static("13"));
        assert_eq!(parse_retry_after_at(&headers, now), None);

        let mut headers = HeaderMap::new();
        let future = httpdate::fmt_http_date(now + Duration::from_secs(23));
        headers.insert(RETRY_AFTER, HeaderValue::from_str(&future).unwrap());
        assert_eq!(
            parse_retry_after_at(&headers, now),
            Some(Duration::from_secs(23))
        );

        let past = httpdate::fmt_http_date(now - Duration::from_secs(1));
        headers.insert(RETRY_AFTER, HeaderValue::from_str(&past).unwrap());
        assert_eq!(parse_retry_after_at(&headers, now), Some(Duration::ZERO));
    }

    #[test]
    fn protocol_retry_classification_respects_fusen_remote_hint() {
        let unavailable = remote_error(StatusCode::SERVICE_UNAVAILABLE, true);
        assert_eq!(
            classify_rpc(&unavailable, WireProtocol::FusenV1),
            FailureClass::Unavailable
        );

        let not_retryable = remote_error(StatusCode::SERVICE_UNAVAILABLE, false);
        assert_eq!(
            classify_rpc(&not_retryable, WireProtocol::FusenV1),
            FailureClass::RemoteFailure
        );
        assert_eq!(
            classify_rpc(&not_retryable, WireProtocol::SpringCloudV1),
            FailureClass::Unavailable
        );
    }

    #[test]
    fn local_retryable_transport_errors_are_not_misclassified_as_invalid_requests() {
        let error = RpcError::internal(
            "transport failed",
            io::Error::new(io::ErrorKind::ConnectionReset, "controlled reset"),
        )
        .mark_retryable();
        assert_eq!(
            classify_rpc(&error, WireProtocol::SpringCloudV1),
            FailureClass::Transport
        );
    }
}
