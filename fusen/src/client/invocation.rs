use super::{
    endpoint_breakers::EndpointBreakerSource,
    runtime::{CLIENT_RUNNING, ClientHttpBinding, ClientRuntimeInner},
    transport::{HttpTransport, TransportFailureKind, circuit_open},
};
use crate::{
    Arguments, Body, Call, Context, Error, ErrorCategory, InstanceRouter, InstanceSnapshot,
    InterceptionStage, Interceptor, LoadBalancer, Response, RouteRequest, Side,
    context::{ContextParts, ResponseAttemptCompletion},
    interceptor::{InterceptorResult, Next, Terminal},
    resilience::{
        FailureClass,
        breaker::{BreakerPermit, BreakerRejection},
        classify::{ClassifiedError, classify_error},
        retry::{RetryDecision, RetryDecisionContext, decide_with_guards, full_jitter_backoff},
    },
    runtime::{
        BoxFuture,
        admission::{AdmissionError, AdmissionGuard},
        deadline::Deadline,
        metrics::SafeMetrics,
    },
    wire::{decode_http_response, encode_request_template, remote_protocol_error},
};
use fusen_contract::{
    EndpointCapabilities, HttpBindingId, HttpVersionPolicy, HttpVersionSet, InstanceId,
    MethodDescriptor, MethodId, ServiceDescriptor, ServiceEndpoint, ServiceInstance, ServiceWeight,
};
use fusen_observability::{
    AdmissionRejectedEvent, AttemptFinishedEvent, InvocationFinishedEvent, InvocationStartedEvent,
    MetricEvent, MetricOutcome, MetricSide,
};
use fusen_register::directory::{Directory, DirectoryState};
use serde::de::DeserializeOwned;
#[cfg(test)]
use serde_json::Value;
use std::{
    collections::HashSet,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant as StdInstant},
};
use tracing::Instrument;

pub(crate) enum EndpointSource {
    Direct {
        endpoint: ServiceEndpoint,
        capabilities: Option<EndpointCapabilities>,
    },
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
    pub binding_id: HttpBindingId,
    pub binding: Arc<ClientHttpBinding>,
    pub http_version_policy: HttpVersionPolicy,
    pub source: EndpointSource,
    pub interceptor: Arc<[Arc<dyn Interceptor>]>,
    pub attempt_interceptor: Arc<[Arc<dyn Interceptor>]>,
    pub routers: Arc<[Arc<dyn InstanceRouter>]>,
    pub load_balancer: Arc<dyn LoadBalancer>,
}

impl ServiceClient {
    /// Executes one typed logical invocation within the logical deadline.
    pub async fn invoke<T, F>(
        &self,
        method_id: MethodId,
        call: Call,
        encode: F,
    ) -> Result<Response<T>, Error>
    where
        T: DeserializeOwned,
        F: FnOnce() -> Result<Arguments, Error> + Send,
    {
        let method = self
            .inner
            .service
            .method(method_id)
            .ok_or_else(|| crate::service::method_not_found(method_id))?;
        let request_id = uuid::Uuid::new_v4().simple().to_string();
        if self.inner.runtime.state.load(Ordering::Acquire) != CLIENT_RUNNING {
            return Err(closed_invocation().with_request_id(request_id));
        }
        let deadline = Deadline::after(self.inner.runtime.config.request_timeout());
        let _admission = acquire_admission(&self.inner.runtime, deadline)
            .await
            .map_err(|error| error.with_request_id(request_id.clone()))?;
        let started = StdInstant::now();
        self.inner
            .runtime
            .metrics
            .record(&MetricEvent::InvocationStarted(
                InvocationStartedEvent::new(
                    MetricSide::Client,
                    self.inner.binding_id.as_str(),
                    None,
                    self.inner.service.selector().service_id(),
                    method.invocation_name(),
                ),
            ));
        let span = tracing::info_span!(
            "fusen.client.invocation",
            request_id = %request_id,
            http_binding = self.inner.binding_id.as_str(),
            service = self.inner.service.selector().service_id(),
            method = method.invocation_name(),
        );
        let invocation_request_id = request_id.clone();
        let attempts_started = Arc::new(AtomicU8::new(0));
        let invocation_attempts = attempts_started.clone();
        let invocation = async move {
            let (headers, extensions) = call.into_parts();
            let arguments = match catch_unwind(AssertUnwindSafe(encode)) {
                Ok(result) => result?,
                Err(_) => {
                    tracing::error!("service invocation argument serialization panicked");
                    return Err(Error::framework(
                        ErrorCategory::Internal,
                        "serialization_panic",
                        "failed to serialize invocation arguments",
                    ));
                }
            };
            let transport = self
                .inner
                .runtime
                .transport()
                .map_err(|_| closed_invocation())?;
            let response_request_id = invocation_request_id.clone();
            let context = Context::new(ContextParts {
                side: Side::Client,
                stage: InterceptionStage::ClientCall,
                request_id: invocation_request_id,
                binding_id: self.inner.binding_id.clone(),
                http_version: None,
                interface: self.inner.service,
                method,
                deadline,
                attempt: None,
                endpoint: None,
                headers,
                extensions,
                arguments: Some(arguments),
                response_limit: self
                    .inner
                    .runtime
                    .config
                    .admission()
                    .max_response_body_bytes(),
                response_wire_overhead: 0,
                response_budget: self.inner.runtime.response_budget.clone(),
            });
            let terminal = InvocationTerminal {
                client: self.inner.as_ref(),
                transport,
                endpoint_breaker_permit: Mutex::new(None),
                attempts_started: invocation_attempts,
            };
            let mut response = match Next::new(&self.inner.interceptor, &terminal)
                .run(context)
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    terminal.succeed_endpoint_breaker();
                    return Err(error);
                }
            };
            let attempts = response
                .attempts()
                .max(terminal.attempts_started.load(Ordering::Acquire));
            response.set_attempts(attempts);
            let remote_result = response.is_wire_origin();
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
                    response.finish_attempt(None);
                    if let Some(permit) = endpoint_permit {
                        permit.succeed();
                    }
                    if let Some(permit) = service_permit {
                        permit.succeed();
                    }
                    value
                }
                Ok(Err(error)) => {
                    let error = if remote_result {
                        remote_protocol_error(
                            "invalid_result",
                            "invocation result does not match the generated return type",
                            &response_request_id,
                        )
                        .with_source(error)
                    } else {
                        Error::invalid_result(
                            "invocation result does not match the generated return type",
                            error,
                        )
                    };
                    return Err(finish_result_failure(
                        &mut response,
                        endpoint_permit,
                        service_permit,
                        error,
                        attempts,
                        remote_result,
                    ));
                }
                Err(_) => {
                    tracing::error!("service invocation result deserialization panicked");
                    let error = if remote_result {
                        remote_protocol_error(
                            "invalid_result",
                            "invocation result does not match the generated return type",
                            &response_request_id,
                        )
                    } else {
                        Error::framework(
                            ErrorCategory::DataLoss,
                            "invalid_result",
                            "invocation result does not match the generated return type",
                        )
                    };
                    return Err(finish_result_failure(
                        &mut response,
                        endpoint_permit,
                        service_permit,
                        error,
                        attempts,
                        remote_result,
                    ));
                }
            };
            let response = response.map(|_| value);
            Ok(response)
        }
        .instrument(span);
        let result = tokio::select! {
            biased;
            () = self.inner.runtime.force_cancel.cancelled() => Err(cancelled()),
            result = deadline.run(invocation) => match result {
                Ok(result) => result,
                Err(_) => Err(deadline_exceeded()),
            }
        }
        .map_err(|error| {
            let attempts = error
                .attempts()
                .max(attempts_started.load(Ordering::Acquire));
            error.with_attempts(attempts).with_request_id(request_id)
        });
        let (outcome, attempts, status_class, error_code) = match &result {
            Ok(response) => (
                MetricOutcome::Success,
                response.attempts(),
                Some(status_class(response.status())),
                None,
            ),
            Err(error) => (
                match error.category() {
                    ErrorCategory::DeadlineExceeded => MetricOutcome::Timeout,
                    ErrorCategory::Cancelled => MetricOutcome::Cancelled,
                    ErrorCategory::ResourceExhausted => MetricOutcome::Rejected,
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
            .record(&MetricEvent::InvocationFinished(
                InvocationFinishedEvent::new(
                    MetricSide::Client,
                    self.inner.binding_id.as_str(),
                    None,
                    self.inner.service.selector().service_id(),
                    method.invocation_name(),
                    outcome,
                    status_class,
                    error_code,
                    started.elapsed(),
                    attempts,
                ),
            ));
        result
    }
}

fn finish_result_failure(
    response: &mut Response<Body>,
    endpoint_permit: Option<BreakerPermit>,
    service_permit: Option<BreakerPermit>,
    error: Error,
    attempts: u8,
    remote_result: bool,
) -> Error {
    let classified = ClassifiedError::new(error.with_attempts(attempts), FailureClass::Protocol);
    let failure = classified.class();
    response.finish_attempt(remote_result.then_some(failure));
    if remote_result {
        if let Some(permit) = endpoint_permit {
            permit.fail(failure);
        }
        if let Some(permit) = service_permit {
            permit.fail(failure);
        }
    } else {
        if let Some(permit) = endpoint_permit {
            permit.succeed();
        }
        if let Some(permit) = service_permit {
            permit.succeed();
        }
    }
    classified.into_error()
}

struct InvocationTerminal<'a> {
    client: &'a ServiceClientInner,
    transport: HttpTransport,
    endpoint_breaker_permit: Mutex<Option<BreakerPermit>>,
    attempts_started: Arc<AtomicU8>,
}

impl Terminal for InvocationTerminal<'_> {
    fn call<'a>(&'a self, context: Context) -> BoxFuture<'a, InterceptorResult> {
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

    async fn execute(&self, context: Context) -> InterceptorResult {
        let service_breaker = self
            .client
            .runtime
            .service_breaker(self.client.service, &self.client.binding_id);
        let service_permit = service_breaker.try_acquire().map_err(|_| circuit_open())?;
        match self.execute_attempts(context).await {
            Ok(AttemptSuccess {
                mut response,
                endpoint_breaker_permit,
                service_breaker_failure,
            }) => {
                if let Some(failure) = service_breaker_failure {
                    service_permit.fail(failure);
                    return Ok(response);
                }
                if let Some(endpoint_breaker_permit) = endpoint_breaker_permit {
                    self.hold_endpoint_breaker(endpoint_breaker_permit);
                    response.track_endpoint_breaker();
                    response.hold_service_breaker(service_permit);
                } else if self.attempts_started.load(Ordering::Acquire) == 0 {
                    service_permit.release_unattempted();
                } else {
                    drop(service_permit);
                }
                Ok(response)
            }
            Err(classified) => {
                if self.attempts_started.load(Ordering::Acquire) == 0 {
                    service_permit.release_unattempted();
                } else {
                    service_permit.fail(classified.class());
                }
                Err(classified.into_error())
            }
        }
    }

    async fn execute_attempts(&self, context: Context) -> Result<AttemptSuccess, ClassifiedError> {
        let mut attempted_endpoints = HashSet::new();
        let mut attempt = 1u8;
        let head = *context.method().http_operation().method() == http::Method::HEAD;
        loop {
            let started = StdInstant::now();
            let mut attempt_context = context.clone();
            attempt_context.set_attempt(attempt);
            let selected = self.select_endpoint(&attempt_context, &attempted_endpoints)?;
            let endpoint_key = selected.instance.endpoint().as_str().to_owned();
            attempt_context.set_stage(InterceptionStage::ClientAttempt);
            attempt_context.set_endpoint(selected.instance.clone());
            if !selected.auto_negotiate {
                attempt_context.set_http_version(selected.http_version);
            }
            attempted_endpoints.insert(endpoint_key.clone());
            let terminal = AttemptTerminal {
                client: self.client,
                transport: &self.transport,
                endpoint: &selected.instance,
                endpoint_key: &endpoint_key,
                attempt,
                started,
                head,
                http_version: selected.http_version,
                auto_negotiate: selected.auto_negotiate,
                invocation_controls: selected.invocation_controls,
                attempts_started: self.attempts_started.as_ref(),
                observation: Mutex::new(AttemptObservation::default()),
            };
            let result = Next::new(&self.client.attempt_interceptor, &terminal)
                .run(attempt_context)
                .await;
            let observation = terminal.observation();
            let failure = match &result {
                Ok(_) => {
                    let mut response = result.expect("successful attempt contains a response");
                    response.seal_attempt_duration(started.elapsed());
                    if let Some(failure) = observation.failure {
                        self.client
                            .runtime
                            .metrics
                            .record(&MetricEvent::AttemptFinished(AttemptFinishedEvent::new(
                                self.client.binding_id.as_str(),
                                attempt_http_version_name(
                                    selected.auto_negotiate,
                                    selected.http_version,
                                    observation.http_version,
                                ),
                                self.client.service.selector().service_id(),
                                context.method().invocation_name(),
                                attempt,
                                if failure == FailureClass::Timeout {
                                    MetricOutcome::Timeout
                                } else {
                                    MetricOutcome::Error
                                },
                                Some(failure_name(failure)),
                                started.elapsed(),
                            )));
                        selected.breaker_permit.fail(failure);
                        response.set_attempts(self.attempts_started.load(Ordering::Acquire));
                        return Ok(AttemptSuccess {
                            response,
                            endpoint_breaker_permit: None,
                            service_breaker_failure: Some(failure),
                        });
                    }
                    response.set_attempts(self.attempts_started.load(Ordering::Acquire));
                    let endpoint_breaker_permit =
                        if self.attempts_started.load(Ordering::Acquire) >= attempt {
                            Some(selected.breaker_permit)
                        } else {
                            selected.breaker_permit.release_unattempted();
                            None
                        };
                    return Ok(AttemptSuccess {
                        response,
                        endpoint_breaker_permit,
                        service_breaker_failure: None,
                    });
                }
                Err(error) => {
                    let failure = observation.failure.unwrap_or_else(|| classify_error(error));
                    if let Some(breaker_failure) = observation.failure {
                        selected.breaker_permit.fail(breaker_failure);
                    } else if observation.transport_succeeded {
                        selected.breaker_permit.succeed();
                    } else if self.attempts_started.load(Ordering::Acquire) < attempt {
                        selected.breaker_permit.release_unattempted();
                    } else {
                        drop(selected.breaker_permit);
                    }
                    failure
                }
            };
            if self.attempts_started.load(Ordering::Acquire) >= attempt {
                self.client
                    .runtime
                    .metrics
                    .record(&MetricEvent::AttemptFinished(AttemptFinishedEvent::new(
                        self.client.binding_id.as_str(),
                        attempt_http_version_name(
                            selected.auto_negotiate,
                            selected.http_version,
                            observation.http_version,
                        ),
                        self.client.service.selector().service_id(),
                        context.method().invocation_name(),
                        attempt,
                        if failure == FailureClass::Timeout {
                            MetricOutcome::Timeout
                        } else {
                            MetricOutcome::Error
                        },
                        Some(failure_name(failure)),
                        started.elapsed(),
                    )));
            }
            let error = result
                .expect_err("failed attempt contains an invocation error")
                .with_attempts(self.attempts_started.load(Ordering::Acquire));
            let retry_after = if observation.failure.is_some() {
                observation.retry_after
            } else {
                error.retry_hint().retry_after()
            };
            let (base, cap) = (
                self.client.runtime.config.retry().backoff_base(),
                self.client.runtime.config.retry().backoff_cap(),
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
                return Err(ClassifiedError::new(error, failure));
            }
            let decision = RetryDecisionContext::new(
                attempt,
                self.client.runtime.config.retry().max_attempts(),
                context.method().allows_retries(),
                failure,
                remaining,
            );
            let budget = self
                .client
                .runtime
                .retry_budget(self.client.service, &self.client.binding_id);
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
                    return Err(ClassifiedError::new(
                        Error::framework(
                            ErrorCategory::Internal,
                            "retry_policy_panic",
                            "retry policy failed",
                        )
                        .with_attempts(self.attempts_started.load(Ordering::Acquire)),
                        failure,
                    ));
                }
                return Err(ClassifiedError::new(error, failure));
            }
            tokio::select! {
                biased;
                () = self.client.runtime.force_cancel.cancelled() => {
                    return Err(ClassifiedError::classify(
                        cancelled().with_attempts(self.attempts_started.load(Ordering::Acquire)),
                    ));
                }
                () = tokio::time::sleep(delay) => {}
            }
            attempt = attempt.saturating_add(1);
        }
    }

    fn select_endpoint(
        &self,
        context: &Context,
        attempted: &HashSet<String>,
    ) -> Result<SelectedEndpoint, Error> {
        let (mut instances, source) = match &self.client.source {
            EndpointSource::Direct {
                endpoint,
                capabilities,
            } => {
                let generic_versions = if endpoint.as_url().scheme() == "https" {
                    HttpVersionSet::ALL
                } else {
                    HttpVersionSet::HTTP_1_1
                };
                (
                    vec![ServiceInstance::new(
                        InstanceId::new("direct").expect("static direct instance ID is valid"),
                        endpoint.clone(),
                        capabilities.clone().unwrap_or_else(|| {
                            EndpointCapabilities::new(
                                generic_versions,
                                [self.client.binding_id.clone()],
                                false,
                            )
                            .expect("direct generic capabilities are valid")
                        }),
                        ServiceWeight::default(),
                    )],
                    EndpointBreakerSource::Direct,
                )
            }
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
        if source == EndpointBreakerSource::Discovery && instances.is_empty() {
            return Err(no_instances());
        }
        instances.retain(|instance| {
            instance
                .capabilities()
                .supports_binding(&self.client.binding_id)
                && select_http_version(
                    self.client.http_version_policy,
                    instance.endpoint(),
                    instance.capabilities(),
                )
                .is_some()
        });
        if instances.is_empty() {
            return Err(no_compatible_endpoint());
        }
        let has_untried = instances
            .iter()
            .any(|instance| !attempted.contains(instance.endpoint().as_str()));
        if has_untried {
            instances.retain(|instance| !attempted.contains(instance.endpoint().as_str()));
        }
        let mut routed = InstanceSnapshot::new(instances);
        for router in self.client.routers.iter() {
            let input = routed.clone();
            let output = match catch_unwind(AssertUnwindSafe(|| {
                router.route(RouteRequest::new(context, routed.clone()))
            })) {
                Ok(Ok(instances)) => instances,
                Ok(Err(error)) => return Err(error),
                Err(_) => {
                    tracing::error!("router panicked and was isolated");
                    return Err(Error::framework(
                        ErrorCategory::Internal,
                        "router_panic",
                        "router failed",
                    ));
                }
            };
            validate_router_output(&input, &output)?;
            routed = output;
        }
        if routed.is_empty() {
            return Err(no_instances());
        }
        let mut eligible = Vec::new();
        let mut permits = Vec::new();
        for instance in routed.iter() {
            let breaker = self.client.runtime.endpoint_breaker(
                self.client.service,
                &self.client.binding_id,
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
                return Err(Error::framework(
                    ErrorCategory::Internal,
                    "invalid_load_balancer_selection",
                    "load balancer returned an out-of-range index",
                ));
            }
            Ok(Err(error)) => return Err(error),
            Err(_) => {
                tracing::error!("load balancer panicked and was isolated");
                return Err(Error::framework(
                    ErrorCategory::Internal,
                    "load_balancer_panic",
                    "load balancer failed",
                ));
            }
        };
        let breaker_permit = permits[index]
            .take()
            .expect("eligible endpoint has a breaker permit");
        let instance = eligible[index].clone();
        let auto_negotiate = should_auto_negotiate(
            self.client.http_version_policy,
            instance.endpoint(),
            instance.capabilities(),
        );
        let http_version = if auto_negotiate {
            http::Version::HTTP_11
        } else {
            select_http_version(
                self.client.http_version_policy,
                instance.endpoint(),
                instance.capabilities(),
            )
            .expect("eligible endpoint has a compatible HTTP version")
        };
        let invocation_controls = instance.capabilities().invocation_controls();
        Ok(SelectedEndpoint {
            instance,
            breaker_permit,
            http_version,
            auto_negotiate,
            invocation_controls,
        })
    }
}

struct AttemptSuccess {
    response: Response<Body>,
    endpoint_breaker_permit: Option<BreakerPermit>,
    service_breaker_failure: Option<FailureClass>,
}

struct AttemptMetricState {
    finished: bool,
    duration: Option<Duration>,
}

struct AttemptMetricCompletion {
    metrics: SafeMetrics,
    binding_id: HttpBindingId,
    http_version: http::Version,
    service: &'static ServiceDescriptor,
    method: &'static MethodDescriptor,
    attempt: u8,
    started: StdInstant,
    state: Mutex<AttemptMetricState>,
}

impl AttemptMetricCompletion {
    fn new(
        metrics: SafeMetrics,
        binding_id: HttpBindingId,
        http_version: http::Version,
        service: &'static ServiceDescriptor,
        method: &'static MethodDescriptor,
        attempt: u8,
        started: StdInstant,
    ) -> Self {
        Self {
            metrics,
            binding_id,
            http_version,
            service,
            method,
            attempt,
            started,
            state: Mutex::new(AttemptMetricState {
                finished: false,
                duration: None,
            }),
        }
    }

    fn record(&self, failure: Option<FailureClass>) {
        let duration = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.finished {
                return;
            }
            state.finished = true;
            state.duration.unwrap_or_else(|| self.started.elapsed())
        };
        let outcome = match failure {
            Some(FailureClass::Timeout) => MetricOutcome::Timeout,
            Some(_) => MetricOutcome::Error,
            None => MetricOutcome::Success,
        };
        self.metrics
            .record(&MetricEvent::AttemptFinished(AttemptFinishedEvent::new(
                self.binding_id.as_str(),
                Some(http_version_name(self.http_version)),
                self.service.selector().service_id(),
                self.method.invocation_name(),
                self.attempt,
                outcome,
                failure.map(failure_name),
                duration,
            )));
    }
}

impl ResponseAttemptCompletion for AttemptMetricCompletion {
    fn seal_duration(&self, duration: Duration) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.finished && state.duration.is_none() {
            state.duration = Some(duration);
        }
    }

    fn finish(&self, failure: Option<FailureClass>) {
        self.record(failure);
    }
}

impl Drop for AttemptMetricCompletion {
    fn drop(&mut self) {
        self.record(None);
    }
}

#[derive(Clone, Copy, Default)]
struct AttemptObservation {
    failure: Option<FailureClass>,
    retry_after: Option<Duration>,
    transport_succeeded: bool,
    http_version: Option<http::Version>,
}

struct AttemptTerminal<'a> {
    client: &'a ServiceClientInner,
    transport: &'a HttpTransport,
    endpoint: &'a ServiceInstance,
    endpoint_key: &'a str,
    attempt: u8,
    started: StdInstant,
    head: bool,
    http_version: http::Version,
    auto_negotiate: bool,
    invocation_controls: bool,
    attempts_started: &'a AtomicU8,
    observation: Mutex<AttemptObservation>,
}

impl AttemptTerminal<'_> {
    fn observe(&self, update: impl FnOnce(&mut AttemptObservation)) {
        update(
            &mut self
                .observation
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
    }

    fn observation(&self) -> AttemptObservation {
        *self
            .observation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

impl Terminal for AttemptTerminal<'_> {
    fn call<'a>(&'a self, context: Context) -> BoxFuture<'a, InterceptorResult> {
        Box::pin(async move {
            let bulkhead = self.client.runtime.endpoint_bulkhead(self.endpoint_key);
            let _bulkhead_permit = bulkhead.try_acquire_owned().map_err(|_| {
                Error::framework(
                    ErrorCategory::ResourceExhausted,
                    "endpoint_overloaded",
                    "selected endpoint concurrency is exhausted",
                )
            })?;
            let template = encode_request_template(
                self.client.binding.request_encoder.as_ref(),
                self.client.service,
                context.method(),
                context
                    .arguments()
                    .expect("client attempt context contains encoded arguments"),
                context.headers(),
                self.client
                    .runtime
                    .config
                    .admission()
                    .max_request_body_bytes(),
                &self.client.runtime.request_budget,
            )?;
            let request = template.to_request(
                self.endpoint.endpoint(),
                self.http_version,
                context.request_id(),
                context.deadline().remaining(),
                self.attempt,
                self.invocation_controls,
                self.client.service,
            )?;
            let attempt_span = tracing::info_span!(
                "fusen.client.attempt",
                request_id = %context.request_id(),
                http_binding = self.client.binding_id.as_str(),
                network_protocol_version = tracing::field::Empty,
                service = self.client.service.selector().service_id(),
                method = context.method().invocation_name(),
                attempt = self.attempt,
                endpoint = %self.endpoint_key,
            );
            if !self.auto_negotiate {
                attempt_span.record(
                    "network_protocol_version",
                    http_version_name(self.http_version),
                );
            }
            self.attempts_started.store(self.attempt, Ordering::Release);
            let sent = tokio::select! {
                biased;
                () = self.client.runtime.force_cancel.cancelled() => {
                    return Err(cancelled());
                }
                result = context.deadline().run(
                    self.transport
                        .send(request, self.auto_negotiate)
                        .instrument(attempt_span.clone())
                ) => result,
            };
            let response = match sent {
                Err(_) => {
                    self.observe(|value| {
                        value.failure = Some(FailureClass::Timeout);
                    });
                    return Err(deadline_exceeded());
                }
                Ok(Err(error)) => {
                    let failure = match error.kind {
                        TransportFailureKind::Connect => FailureClass::Connect,
                        TransportFailureKind::Io => FailureClass::Transport,
                    };
                    self.observe(|value| {
                        value.failure = Some(failure);
                    });
                    return Err(error.into_error());
                }
                Ok(Ok(response)) => response,
            };
            let response_http_version = response.version();
            attempt_span.record(
                "network_protocol_version",
                http_version_name(response_http_version),
            );
            self.observe(|value| value.http_version = Some(response_http_version));
            match context
                .deadline()
                .run(
                    decode_http_response(
                        self.client.binding.response_decoder.as_ref(),
                        self.client.binding.error_decoder.as_ref(),
                        self.head,
                        context.method(),
                        context.request_id(),
                        response,
                        self.client
                            .runtime
                            .config
                            .admission()
                            .max_response_body_bytes(),
                        &self.client.runtime.response_budget,
                        self.invocation_controls,
                    )
                    .instrument(attempt_span),
                )
                .await
            {
                Err(_) => {
                    self.observe(|value| {
                        value.failure = Some(FailureClass::Timeout);
                    });
                    Err(deadline_exceeded())
                }
                Ok(Err(error)) => {
                    let failure = classify_error(&error);
                    self.observe(|value| {
                        value.failure = Some(failure);
                        value.retry_after = error.retry_hint().retry_after();
                    });
                    Err(error)
                }
                Ok(Ok(mut response)) => {
                    response.mark_wire_origin();
                    response.hold_attempt_completion(Arc::new(AttemptMetricCompletion::new(
                        self.client.runtime.metrics.clone(),
                        self.client.binding_id.clone(),
                        response_http_version,
                        self.client.service,
                        context.method(),
                        self.attempt,
                        self.started,
                    )));
                    self.observe(|value| value.transport_succeeded = true);
                    Ok(response)
                }
            }
        })
    }
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
    http_version: http::Version,
    auto_negotiate: bool,
    invocation_controls: bool,
}

fn validate_router_output(
    input: &InstanceSnapshot,
    output: &InstanceSnapshot,
) -> Result<(), Error> {
    let mut matched = vec![false; input.len()];
    for instance in output.iter() {
        let Some(index) = input.iter().enumerate().find_map(|(index, candidate)| {
            (!matched[index] && same_instance(candidate, instance)).then_some(index)
        }) else {
            return Err(Error::framework(
                ErrorCategory::Internal,
                "invalid_router_output",
                "instance router must return a non-duplicated subset of its input snapshot",
            ));
        };
        matched[index] = true;
    }
    Ok(())
}

fn same_instance(left: &ServiceInstance, right: &ServiceInstance) -> bool {
    left.instance_id() == right.instance_id()
        && left.endpoint() == right.endpoint()
        && left.capabilities() == right.capabilities()
        && left.weight() == right.weight()
        && left.metadata() == right.metadata()
}

fn select_http_version(
    policy: HttpVersionPolicy,
    endpoint: &ServiceEndpoint,
    capabilities: &EndpointCapabilities,
) -> Option<http::Version> {
    let versions = capabilities.http_versions();
    let secure = endpoint.as_url().scheme() == "https";
    match policy {
        HttpVersionPolicy::Auto => {
            if secure && versions.contains(http::Version::HTTP_2) {
                Some(http::Version::HTTP_2)
            } else if versions.contains(http::Version::HTTP_11) {
                Some(http::Version::HTTP_11)
            } else {
                None
            }
        }
        HttpVersionPolicy::Http1 => versions
            .contains(http::Version::HTTP_11)
            .then_some(http::Version::HTTP_11),
        HttpVersionPolicy::Http2 => {
            (secure && versions.contains(http::Version::HTTP_2)).then_some(http::Version::HTTP_2)
        }
        HttpVersionPolicy::H2c => {
            (!secure && versions.contains(http::Version::HTTP_2)).then_some(http::Version::HTTP_2)
        }
        _ => None,
    }
}

fn should_auto_negotiate(
    policy: HttpVersionPolicy,
    endpoint: &ServiceEndpoint,
    capabilities: &EndpointCapabilities,
) -> bool {
    let versions = capabilities.http_versions();
    policy == HttpVersionPolicy::Auto
        && endpoint.as_url().scheme() == "https"
        && versions.contains(http::Version::HTTP_11)
        && versions.contains(http::Version::HTTP_2)
}

async fn acquire_admission(
    runtime: &Arc<ClientRuntimeInner>,
    deadline: Deadline,
) -> Result<AdmissionGuard, Error> {
    match runtime.admission.try_enter() {
        Ok(guard) => return Ok(guard),
        Err(AdmissionError::Draining) => return Err(closed_invocation()),
        Err(AdmissionError::Overloaded) => {}
    }
    let Some(queue) = &runtime.queue_slots else {
        runtime.metrics.record(&MetricEvent::AdmissionRejected(
            AdmissionRejectedEvent::new(MetricSide::Client, "concurrency"),
        ));
        return Err(Error::framework(
            ErrorCategory::ResourceExhausted,
            "overloaded",
            "client logical invocation concurrency is exhausted",
        ));
    };
    let queue_permit = queue.clone().try_acquire_owned().map_err(|_| {
        Error::framework(
            ErrorCategory::ResourceExhausted,
            "admission_queue_full",
            "client admission queue is full",
        )
    })?;
    let queue_deadline = deadline.min(Deadline::after(
        runtime.config.admission().queue().max_wait(),
    ));
    let result = queue_deadline.run(runtime.admission.enter()).await;
    drop(queue_permit);
    match result {
        Ok(Ok(guard)) => Ok(guard),
        Ok(Err(_)) => Err(closed_invocation()),
        Err(_) if deadline.is_elapsed() => Err(deadline_exceeded()),
        Err(_) => Err(Error::framework(
            ErrorCategory::ResourceExhausted,
            "admission_queue_timeout",
            "client admission queue wait elapsed",
        )),
    }
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

fn no_instances() -> Error {
    Error::framework(
        ErrorCategory::Unavailable,
        "no_instances",
        "discovery has no currently routable service instances",
    )
}

fn no_compatible_endpoint() -> Error {
    Error::framework(
        ErrorCategory::Unavailable,
        "no_compatible_endpoint",
        "no endpoint is compatible with the selected HTTP binding and version policy",
    )
}

fn closed_invocation() -> Error {
    Error::framework(
        ErrorCategory::Unavailable,
        "client_closed",
        "client runtime is draining or closed",
    )
}

fn cancelled() -> Error {
    Error::framework(
        ErrorCategory::Cancelled,
        "cancelled",
        "service invocation was cancelled",
    )
}

fn deadline_exceeded() -> Error {
    Error::framework(
        ErrorCategory::DeadlineExceeded,
        "deadline_exceeded",
        "service invocation deadline elapsed",
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

fn http_version_name(version: http::Version) -> &'static str {
    match version {
        http::Version::HTTP_09 => "0.9",
        http::Version::HTTP_10 => "1.0",
        http::Version::HTTP_11 => "1.1",
        http::Version::HTTP_2 => "2",
        http::Version::HTTP_3 => "3",
        _ => "unknown",
    }
}

fn attempt_http_version_name(
    auto_negotiate: bool,
    selected: http::Version,
    observed: Option<http::Version>,
) -> Option<&'static str> {
    observed
        .map(http_version_name)
        .or_else(|| (!auto_negotiate).then(|| http_version_name(selected)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Arguments, BreakerThreshold, BufferedResponse, Call, CircuitBreakerConfig,
        ClientAdmissionConfig, ClientConfig, ClientRuntime, EncodedRequest, ErrorCode,
        ErrorDecoder, ErrorKind, ErrorOrigin, InstanceRouter, InstanceSnapshot, InterceptorFuture,
        LoadBalancer, RequestEncoder, RequestEncoding, ResponseDecoder, RetryConfig, RetryHint,
        RouteRequest,
        interceptor::erase_interceptor,
        resilience::breaker::BreakerState,
        runtime::budget::ByteBudget,
        wire::{
            ATTEMPT, JSON_CONTENT_TYPE, PROBLEM_CONTENT_TYPE, ProblemDetails, REQUEST_ID,
            SERVICE_GROUP, TIMEOUT_MS,
        },
    };
    use bytes::Bytes;
    use fusen_contract::{
        EndpointCapabilities, HttpBindingId, HttpOperation, HttpParameter,
        HttpParameterCardinality, HttpParameterSource, HttpVersionPolicy, HttpVersionSet,
        MethodDescriptor, MethodId, ServiceInstance, ServiceSelector,
    };
    use fusen_observability::MetricsRecorder;
    use fusen_register::directory::{DirectoryPublisher, DirectoryState, directory};
    use http::{
        HeaderMap, HeaderValue, Method, Request, Response as HttpResponse, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE, RETRY_AFTER},
    };
    use http_body_util::{BodyExt, Full};
    use hyper::{body::Incoming, service::service_fn};
    use hyper_util::rt::TokioIo;
    use serde::Deserialize;
    use serde_json::json;
    use std::{
        convert::Infallible,
        sync::{
            Arc, OnceLock,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::{mpsc, oneshot},
        task::JoinHandle,
    };

    fn empty_arguments() -> Result<Arguments, Error> {
        Ok(Arguments::new())
    }

    #[test]
    fn https_auto_negotiates_when_endpoint_supports_both_http_versions() {
        let binding = HttpBindingId::default();
        let both =
            EndpointCapabilities::new(HttpVersionSet::ALL, [binding.clone()], false).unwrap();
        let h2_only = EndpointCapabilities::new(HttpVersionSet::HTTP_2, [binding], false).unwrap();
        let https: ServiceEndpoint = "https://service.example".parse().unwrap();
        let http: ServiceEndpoint = "http://service.example".parse().unwrap();

        assert!(should_auto_negotiate(
            HttpVersionPolicy::Auto,
            &https,
            &both
        ));
        assert!(!should_auto_negotiate(
            HttpVersionPolicy::Auto,
            &https,
            &h2_only
        ));
        assert!(!should_auto_negotiate(
            HttpVersionPolicy::Auto,
            &http,
            &both
        ));
        assert!(!should_auto_negotiate(
            HttpVersionPolicy::Http2,
            &https,
            &both
        ));
    }

    #[derive(Debug)]
    struct CapturedAttempt {
        endpoint: &'static str,
        request_id: String,
        attempt: u8,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct CapturedHttpRequest {
        method: Method,
        version: http::Version,
        uri: http::Uri,
        headers: HeaderMap,
        body: Bytes,
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
                MetricEvent::InvocationStarted(event) if event.side() == MetricSide::Client => {
                    self.started.fetch_add(1, Ordering::SeqCst);
                }
                MetricEvent::InvocationFinished(event)
                    if event.side() == MetricSide::Client
                        && event.outcome() == MetricOutcome::Success =>
                {
                    self.succeeded.fetch_add(1, Ordering::SeqCst);
                }
                MetricEvent::InvocationFinished(event) if event.side() == MetricSide::Client => {
                    self.failed.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
    }

    type FinishedAttemptMetric = (u8, MetricOutcome, Option<String>);

    #[derive(Clone, Default)]
    struct AttemptMetrics {
        finished: Arc<Mutex<Vec<FinishedAttemptMetric>>>,
    }

    impl MetricsRecorder for AttemptMetrics {
        fn record(&self, event: &MetricEvent<'_>) {
            if let MetricEvent::AttemptFinished(event) = event {
                self.finished
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push((
                        event.attempt(),
                        event.outcome(),
                        event.failure_class().map(str::to_owned),
                    ));
            }
        }
    }

    #[derive(Clone, Default)]
    struct AttemptVersionMetrics {
        finished: Arc<Mutex<Vec<Option<String>>>>,
    }

    impl MetricsRecorder for AttemptVersionMetrics {
        fn record(&self, event: &MetricEvent<'_>) {
            if let MetricEvent::AttemptFinished(event) = event {
                self.finished
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(event.http_version().map(str::to_owned));
            }
        }
    }

    #[derive(Clone, Default)]
    struct CombinedMetrics {
        invocation: InvocationMetrics,
        attempt: AttemptMetrics,
    }

    impl MetricsRecorder for CombinedMetrics {
        fn record(&self, event: &MetricEvent<'_>) {
            self.invocation.record(event);
            self.attempt.record(event);
        }
    }

    #[derive(Debug, Deserialize)]
    struct StructuredResult {
        _value: String,
    }

    #[derive(Clone, Copy)]
    struct FirstEndpoint;

    impl LoadBalancer for FirstEndpoint {
        fn select(&self, _context: &Context, instances: &InstanceSnapshot) -> Result<usize, Error> {
            assert!(
                !instances.is_empty(),
                "fixture always publishes an endpoint"
            );
            Ok(0)
        }
    }

    struct PanickingRouter;

    impl InstanceRouter for PanickingRouter {
        fn route(&self, _request: RouteRequest<'_>) -> Result<InstanceSnapshot, Error> {
            panic!("private router panic")
        }
    }

    struct InjectingRouter;

    impl InstanceRouter for InjectingRouter {
        fn route(&self, _request: RouteRequest<'_>) -> Result<InstanceSnapshot, Error> {
            Ok(InstanceSnapshot::new(vec![ServiceInstance::new(
                InstanceId::new("injected").unwrap(),
                "http://127.0.0.1:9".parse().unwrap(),
                EndpointCapabilities::default(),
                ServiceWeight::default(),
            )]))
        }
    }

    struct EmptyRouter;

    impl InstanceRouter for EmptyRouter {
        fn route(&self, _request: RouteRequest<'_>) -> Result<InstanceSnapshot, Error> {
            Ok(InstanceSnapshot::new(Vec::new()))
        }
    }

    struct PanickingLoadBalancer;

    impl LoadBalancer for PanickingLoadBalancer {
        fn select(
            &self,
            _context: &Context,
            _instances: &InstanceSnapshot,
        ) -> Result<usize, Error> {
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

    impl crate::Interceptor for ReplaceRemoteResult {
        fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
            Box::pin(async move {
                let local_response = context.clone();
                drop(next.run(context).await?);
                local_response.respond("interceptor-result")
            })
        }
    }

    struct MutateRemoteResult;

    impl crate::Interceptor for MutateRemoteResult {
        fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
            Box::pin(async move {
                let local_response = context.clone();
                let mut response = next.run(context).await?;
                *response.body_mut() = local_response.respond("interceptor-result")?.into_body();
                Ok(response)
            })
        }
    }

    struct MapRemoteResult;

    impl crate::Interceptor for MapRemoteResult {
        fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
            Box::pin(async move {
                let local_response = context.clone();
                let response = next.run(context).await?;
                let replacement = local_response.respond("interceptor-result")?.into_body();
                Ok(response.map(|_| replacement))
            })
        }
    }

    struct RecoverAttemptError;

    impl crate::Interceptor for RecoverAttemptError {
        fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
            Box::pin(async move {
                let fallback_context = context.clone();
                match next.run(context).await {
                    Ok(response) => Ok(response),
                    Err(_) => fallback_context.respond("fallback"),
                }
            })
        }
    }

    struct ShortCircuitAttempt;

    impl crate::Interceptor for ShortCircuitAttempt {
        fn intercept<'a>(&'a self, context: Context, _next: Next<'a>) -> InterceptorFuture<'a> {
            Box::pin(async move {
                if context.headers().contains_key("x-short-circuit-error") {
                    Err(Error::local(
                        ErrorCategory::InvalidArgument,
                        "short_circuit_error",
                        "attempt interceptor rejected the invocation",
                    )
                    .expect("the static error metadata is valid"))
                } else {
                    context.respond("short-circuit")
                }
            })
        }
    }

    #[derive(Clone)]
    struct ObserveAutoHttpVersion {
        calls: Arc<AtomicUsize>,
    }

    impl crate::Interceptor for ObserveAutoHttpVersion {
        fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
            assert_eq!(context.stage(), InterceptionStage::ClientAttempt);
            assert_eq!(context.http_version(), None);
            assert_eq!(
                context.endpoint().unwrap().endpoint().as_url().scheme(),
                "https"
            );
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { next.run(context).await })
        }
    }

    struct PanickingRequestEncoder;

    impl RequestEncoder for PanickingRequestEncoder {
        fn encode(&self, _request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
            panic!("private request encoder panic")
        }
    }

    struct ReservedControlHeaderEncoder;

    impl RequestEncoder for ReservedControlHeaderEncoder {
        fn encode(&self, _request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
            let mut headers = HeaderMap::new();
            headers.insert(REQUEST_ID, HeaderValue::from_static("forged-request-id"));
            Ok(EncodedRequest::new(
                Method::GET,
                "/call",
                headers,
                Bytes::new(),
            ))
        }
    }

    struct PanickingResponseDecoder;

    impl ResponseDecoder for PanickingResponseDecoder {
        fn decode(
            &self,
            _method: &'static MethodDescriptor,
            _response: BufferedResponse,
        ) -> Result<Response<Body>, Error> {
            panic!("private response decoder panic")
        }
    }

    struct ExpandingResponseDecoder;

    impl ResponseDecoder for ExpandingResponseDecoder {
        fn decode(
            &self,
            _method: &'static MethodDescriptor,
            _response: BufferedResponse,
        ) -> Result<Response<Body>, Error> {
            Ok(Response::new(Body::from_bytes(Bytes::from(vec![b'x'; 33]))))
        }
    }

    struct PanickingErrorDecoder;

    impl ErrorDecoder for PanickingErrorDecoder {
        fn decode(&self, _method: &'static MethodDescriptor, _response: BufferedResponse) -> Error {
            panic!("private error decoder panic")
        }
    }

    #[derive(Clone)]
    struct InspectingErrorDecoder {
        headers: Arc<Mutex<Option<HeaderMap>>>,
    }

    impl ErrorDecoder for InspectingErrorDecoder {
        fn decode(&self, _method: &'static MethodDescriptor, response: BufferedResponse) -> Error {
            *self
                .headers
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(response.headers().clone());
            Error::local(
                ErrorCategory::Unavailable,
                "custom_remote_unavailable",
                "custom binding decoded a remote unavailable response",
            )
            .unwrap()
            .with_retry_hint(RetryHint::Retryable)
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct HeadErrorObservation {
        method: Method,
        status: StatusCode,
        content_length: String,
        body: Bytes,
    }

    #[derive(Clone)]
    struct InspectingHeadErrorDecoder {
        observation: Arc<Mutex<Option<HeadErrorObservation>>>,
    }

    impl ErrorDecoder for InspectingHeadErrorDecoder {
        fn decode(&self, method: &'static MethodDescriptor, response: BufferedResponse) -> Error {
            *self
                .observation
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(HeadErrorObservation {
                method: method.http_operation().method().clone(),
                status: response.status(),
                content_length: response
                    .headers()
                    .get(CONTENT_LENGTH)
                    .expect("the HEAD response exposes its semantic Content-Length")
                    .to_str()
                    .unwrap()
                    .to_owned(),
                body: response.body().clone(),
            });
            Error::local(
                ErrorCategory::Unavailable,
                "custom_head_unavailable",
                "custom binding decoded a remote HEAD failure",
            )
            .unwrap()
        }
    }

    fn replay_service() -> &'static ServiceDescriptor {
        Box::leak(Box::new(
            ServiceDescriptor::new(
                ServiceSelector::new("replay", None, None).unwrap(),
                vec![
                    MethodDescriptor::new(
                        MethodId::new(0),
                        "call",
                        HttpOperation::new(
                            Method::PUT,
                            "/call",
                            vec![
                                HttpParameter::new(
                                    "value",
                                    HttpParameterSource::Body,
                                    HttpParameterCardinality::Scalar,
                                )
                                .unwrap(),
                            ],
                            "application/json",
                            "application/json",
                        )
                        .unwrap(),
                    )
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
                        HttpOperation::new(
                            Method::GET,
                            "/call",
                            Vec::new(),
                            "application/json",
                            "application/json",
                        )
                        .unwrap(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap()
        })
    }

    fn head_service() -> &'static ServiceDescriptor {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        SERVICE.get_or_init(|| {
            ServiceDescriptor::new(
                ServiceSelector::new("head", None, None).unwrap(),
                vec![
                    MethodDescriptor::new(
                        MethodId::new(0),
                        "probe",
                        HttpOperation::new(
                            Method::HEAD,
                            "/probe",
                            Vec::new(),
                            "application/json",
                            "application/json",
                        )
                        .unwrap(),
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
        resilience_config_with_close_successes(request_timeout, retry, endpoint_close_successes, 1)
    }

    fn resilience_config_with_close_successes(
        request_timeout: Duration,
        retry: RetryConfig,
        endpoint_close_successes: u32,
        service_close_successes: u32,
    ) -> ClientConfig {
        let endpoint = BreakerThreshold::endpoint_builder()
            .minimum_samples(1)
            .failure_ratio(1.0)
            .close_successes(endpoint_close_successes)
            .build()
            .unwrap();
        let service = BreakerThreshold::service_builder()
            .minimum_samples(1)
            .failure_ratio(1.0)
            .half_open_probes(1)
            .close_successes(service_close_successes)
            .build()
            .unwrap();
        let circuit_breaker = CircuitBreakerConfig::builder()
            .endpoint(endpoint)
            .service(service)
            .build()
            .unwrap();
        ClientConfig::builder()
            .request_timeout(request_timeout)
            .retry(retry)
            .circuit_breaker(circuit_breaker)
            .build()
            .unwrap()
    }

    fn discovered_client(
        runtime: &ClientRuntime,
        instances: Vec<ServiceInstance>,
    ) -> (DirectoryPublisher, ServiceClient) {
        let (publisher, directory) = directory();
        runtime
            .inner
            .endpoint_breakers
            .replace_discovery(resilience_service().selector(), &instances);
        publisher.publish_ready(instances).unwrap();
        let binding_id = HttpBindingId::default();
        let binding = runtime
            .inner
            .http_bindings
            .get(&binding_id)
            .expect("default JSON binding is registered")
            .clone();
        let client = ServiceClient {
            inner: Arc::new(ServiceClientInner {
                runtime: runtime.inner.clone(),
                service: resilience_service(),
                binding_id,
                binding,
                http_version_policy: HttpVersionPolicy::Auto,
                source: EndpointSource::Discovery(directory),
                interceptor: Arc::from(Vec::<Arc<dyn crate::Interceptor>>::new()),
                attempt_interceptor: Arc::from(Vec::<Arc<dyn crate::Interceptor>>::new()),
                routers: Arc::from(Vec::<Arc<dyn InstanceRouter>>::new()),
                load_balancer: Arc::new(FirstEndpoint),
            }),
        };
        (publisher, client)
    }

    fn direct_client(runtime: &ClientRuntime, endpoint: ServiceEndpoint) -> ServiceClient {
        let binding_id = HttpBindingId::default();
        let binding = runtime
            .inner
            .http_bindings
            .get(&binding_id)
            .expect("default JSON binding is registered")
            .clone();
        ServiceClient {
            inner: Arc::new(ServiceClientInner {
                runtime: runtime.inner.clone(),
                service: resilience_service(),
                binding_id: binding_id.clone(),
                binding,
                http_version_policy: HttpVersionPolicy::Auto,
                source: EndpointSource::Direct {
                    endpoint,
                    capabilities: Some(
                        EndpointCapabilities::new(HttpVersionSet::HTTP_1_1, [binding_id], true)
                            .unwrap(),
                    ),
                },
                interceptor: Arc::from(Vec::<Arc<dyn crate::Interceptor>>::new()),
                attempt_interceptor: Arc::from(Vec::<Arc<dyn crate::Interceptor>>::new()),
                routers: Arc::from(Vec::<Arc<dyn InstanceRouter>>::new()),
                load_balancer: Arc::new(FirstEndpoint),
            }),
        }
    }

    fn direct_client_with_binding(
        runtime: &ClientRuntime,
        endpoint: ServiceEndpoint,
        binding_id: HttpBindingId,
    ) -> ServiceClient {
        direct_client_for_service_with_binding(runtime, endpoint, binding_id, resilience_service())
    }

    fn direct_head_client_with_binding(
        runtime: &ClientRuntime,
        endpoint: ServiceEndpoint,
        binding_id: HttpBindingId,
    ) -> ServiceClient {
        direct_client_for_service_with_binding(runtime, endpoint, binding_id, head_service())
    }

    fn direct_client_for_service_with_binding(
        runtime: &ClientRuntime,
        endpoint: ServiceEndpoint,
        binding_id: HttpBindingId,
        service: &'static ServiceDescriptor,
    ) -> ServiceClient {
        let binding = runtime
            .inner
            .http_bindings
            .get(&binding_id)
            .expect("custom HTTP binding is registered")
            .clone();
        ServiceClient {
            inner: Arc::new(ServiceClientInner {
                runtime: runtime.inner.clone(),
                service,
                binding_id: binding_id.clone(),
                binding,
                http_version_policy: HttpVersionPolicy::Auto,
                source: EndpointSource::Direct {
                    endpoint,
                    capabilities: Some(
                        EndpointCapabilities::new(HttpVersionSet::HTTP_1_1, [binding_id], true)
                            .unwrap(),
                    ),
                },
                interceptor: Arc::from(Vec::<Arc<dyn crate::Interceptor>>::new()),
                attempt_interceptor: Arc::from(Vec::<Arc<dyn crate::Interceptor>>::new()),
                routers: Arc::from(Vec::<Arc<dyn InstanceRouter>>::new()),
                load_balancer: Arc::new(FirstEndpoint),
            }),
        }
    }

    fn assert_codec_panic(error: &Error, attempts: u8) {
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.origin(), ErrorOrigin::Local);
        assert_eq!(error.category(), ErrorCategory::Internal);
        assert_eq!(error.code().as_str(), "codec_panic");
        assert_eq!(error.attempts(), attempts);
        assert!(error.request_id().is_some());
        assert!(!error.message().contains("private"));
    }

    fn assert_binding_breakers_unsampled(
        runtime: &ClientRuntime,
        endpoint: &ServiceEndpoint,
        binding_id: &HttpBindingId,
    ) {
        assert_service_binding_breakers_unsampled(
            runtime,
            resilience_service(),
            endpoint,
            binding_id,
        );
    }

    fn assert_service_binding_breakers_unsampled(
        runtime: &ClientRuntime,
        service: &'static ServiceDescriptor,
        endpoint: &ServiceEndpoint,
        binding_id: &HttpBindingId,
    ) {
        let endpoint = runtime
            .inner
            .endpoint_breaker(
                service,
                binding_id,
                EndpointBreakerSource::Direct,
                endpoint.as_str(),
            )
            .snapshot();
        assert_eq!(endpoint.state, BreakerState::Closed);
        assert_eq!((endpoint.samples, endpoint.failures), (0, 0));

        let service = runtime
            .inner
            .service_breaker(service, binding_id)
            .snapshot();
        assert_eq!(service.state, BreakerState::Closed);
        assert_eq!((service.samples, service.failures), (0, 0));
    }

    fn instance(id: &str, endpoint: ServiceEndpoint) -> ServiceInstance {
        ServiceInstance::new(
            InstanceId::new(id).unwrap(),
            endpoint,
            EndpointCapabilities::new(HttpVersionSet::HTTP_1_1, [HttpBindingId::default()], true)
                .unwrap(),
            ServiceWeight::default(),
        )
    }

    async fn capture_request(
        endpoint: &'static str,
        request: Request<Incoming>,
        captured: &mpsc::UnboundedSender<CapturedAttempt>,
    ) -> String {
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
                request_id: request_id.clone(),
                attempt,
            })
            .unwrap();
        request_id
    }

    async fn spawn_broken_body_endpoint(
        captured: mpsc::UnboundedSender<CapturedAttempt>,
    ) -> (ServiceEndpoint, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let fixture = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::with_capacity(2048);
            let mut buffer = [0u8; 2048];
            let head_end = loop {
                if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    break index + 4;
                }
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "request ended before its headers completed");
                request.extend_from_slice(&buffer[..read]);
                assert!(request.len() <= 16 * 1024, "request headers are bounded");
            };
            let head = std::str::from_utf8(&request[..head_end - 4]).unwrap();
            let mut lines = head.split("\r\n");
            assert!(lines.next().is_some(), "request contains a start line");
            let mut headers = HeaderMap::new();
            for line in lines {
                let (name, value) = line.split_once(':').unwrap();
                headers.append(
                    http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    HeaderValue::from_str(value.trim()).unwrap(),
                );
            }
            let content_length = headers
                .get(CONTENT_LENGTH)
                .map(|value| value.to_str().unwrap().parse::<usize>().unwrap())
                .unwrap_or(0);
            while request.len() < head_end + content_length {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "request ended before its body completed");
                request.extend_from_slice(&buffer[..read]);
            }
            let request_id = headers
                .get(REQUEST_ID)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            let attempt = headers
                .get(ATTEMPT)
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap();
            captured
                .send(CapturedAttempt {
                    endpoint: "broken",
                    request_id: request_id.clone(),
                    attempt,
                })
                .unwrap();

            let response_head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {JSON_CONTENT_TYPE}\r\nContent-Length: 8\r\nx-request-id: {request_id}\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(response_head.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            stream.write_all(b"\"").await.unwrap();
            stream.flush().await.unwrap();
            stream.shutdown().await.unwrap();
        });
        (endpoint, fixture)
    }

    async fn spawn_gated_retryable_endpoint(
        captured: mpsc::UnboundedSender<CapturedAttempt>,
        failure_release: oneshot::Receiver<()>,
    ) -> (ServiceEndpoint, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let failure_release = Arc::new(Mutex::new(Some(failure_release)));
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let service = service_fn(move |request| {
                let captured = captured.clone();
                let failure_release = failure_release
                    .lock()
                    .unwrap()
                    .take()
                    .expect("gated endpoint receives one request");
                async move {
                    let request_id = capture_request("old", request, &captured).await;
                    failure_release
                        .await
                        .expect("test releases the first failed attempt");
                    let body = serde_json::to_vec(&json!({
                        "type": "urn:fusen:error:unavailable:fixture_unavailable",
                        "title": "Service Unavailable",
                        "status": 503,
                        "detail": "retry later",
                        "code": "fixture_unavailable",
                        "request_id": request_id,
                        "retryable": true
                    }))
                    .unwrap();
                    Ok::<_, Infallible>(
                        HttpResponse::builder()
                            .status(StatusCode::SERVICE_UNAVAILABLE)
                            .header(CONTENT_TYPE, PROBLEM_CONTENT_TYPE)
                            .header(REQUEST_ID, request_id)
                            .body(Full::new(Bytes::from(body)))
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
                    let request_id = capture_request(endpoint_name, request, &captured).await;
                    let body = if status.is_success() {
                        body
                    } else if let Ok(mut problem) =
                        serde_json::from_slice::<serde_json::Value>(&body)
                    {
                        problem["request_id"] = serde_json::Value::String(request_id.clone());
                        Bytes::from(serde_json::to_vec(&problem).unwrap())
                    } else {
                        body
                    };
                    let mut response = HttpResponse::builder()
                        .status(status)
                        .header(
                            CONTENT_TYPE,
                            if status.is_success() {
                                JSON_CONTENT_TYPE
                            } else {
                                PROBLEM_CONTENT_TYPE
                            },
                        )
                        .header(REQUEST_ID, request_id);
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

    async fn spawn_decoder_header_endpoint(
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
                    let request_id =
                        capture_request("custom-error-decoder", request, &captured).await;
                    Ok::<_, Infallible>(
                        HttpResponse::builder()
                            .status(StatusCode::SERVICE_UNAVAILABLE)
                            .header(CONTENT_TYPE, PROBLEM_CONTENT_TYPE)
                            .header(RETRY_AFTER, "0")
                            .header(REQUEST_ID, request_id)
                            .header(ATTEMPT, "99")
                            .header(TIMEOUT_MS, "1")
                            .header(SERVICE_GROUP, "forged")
                            .header("x-codec-visible", "yes")
                            .body(Full::new(Bytes::from_static(b"custom error")))
                            .unwrap(),
                    )
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

    async fn spawn_http10_response_endpoint(
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
                    let request_id = capture_request("http-1.0-response", request, &captured).await;
                    let mut response = HttpResponse::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, JSON_CONTENT_TYPE)
                        .header(REQUEST_ID, request_id)
                        .body(Full::new(Bytes::from_static(b"null")))
                        .unwrap();
                    *response.version_mut() = http::Version::HTTP_10;
                    Ok::<_, Infallible>(response)
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

    async fn spawn_head_error_endpoint(
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
                    assert_eq!(request.method(), Method::HEAD);
                    let request_id =
                        capture_request("head-error-decoder", request, &captured).await;
                    Ok::<_, Infallible>(
                        HttpResponse::builder()
                            .status(StatusCode::SERVICE_UNAVAILABLE)
                            .header(CONTENT_LENGTH, "1048576")
                            .header(REQUEST_ID, request_id)
                            .header("x-codec-visible", "yes")
                            .body(Full::new(Bytes::from(vec![b'x'; 1_048_576])))
                            .unwrap(),
                    )
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

    async fn spawn_request_parity_endpoint() -> (
        ServiceEndpoint,
        mpsc::UnboundedReceiver<CapturedHttpRequest>,
        JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let (captured_tx, captured_rx) = mpsc::unbounded_channel();
        let fixture = tokio::spawn(async move {
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                let captured = captured_tx.clone();
                let service = service_fn(move |request: Request<Incoming>| {
                    let captured = captured.clone();
                    async move {
                        let (parts, body) = request.into_parts();
                        let body = body.collect().await.unwrap().to_bytes();
                        captured
                            .send(CapturedHttpRequest {
                                method: parts.method,
                                version: parts.version,
                                uri: parts.uri,
                                headers: parts.headers,
                                body,
                            })
                            .unwrap();
                        Ok::<_, Infallible>(
                            HttpResponse::builder()
                                .status(StatusCode::OK)
                                .header(CONTENT_TYPE, JSON_CONTENT_TYPE)
                                .body(Full::new(Bytes::from_static(b"null")))
                                .unwrap(),
                        )
                    }
                });
                let mut builder = hyper::server::conn::http1::Builder::new();
                builder.keep_alive(false);
                builder
                    .serve_connection(TokioIo::new(stream), service)
                    .await
                    .unwrap();
            }
        });
        (endpoint, captured_rx, fixture)
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
        let budget = ByteBudget::new(1024);
        let template = encode_request_template(
            &crate::wire::JsonCodec,
            service,
            method,
            &arguments,
            &application_headers,
            1024,
            &budget,
        )
        .unwrap();
        let body_len = template.body.len();
        assert_eq!(budget.used(), body_len);

        let first = template
            .to_request(
                &endpoint,
                http::Version::HTTP_11,
                "same-request",
                Duration::from_millis(1500),
                1,
                true,
                service,
            )
            .unwrap();
        let second = template
            .to_request(
                &endpoint,
                http::Version::HTTP_11,
                "same-request",
                Duration::from_millis(900),
                2,
                true,
                service,
            )
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

    #[tokio::test]
    async fn direct_and_discovery_emit_the_same_http_request_for_equal_capabilities() {
        let (endpoint, mut captured, fixture) = spawn_request_parity_endpoint().await;
        let runtime = ClientRuntime::builder().build().unwrap();
        let capabilities =
            EndpointCapabilities::new(HttpVersionSet::HTTP_1_1, [HttpBindingId::default()], false)
                .unwrap();
        let mut direct = direct_client(&runtime, endpoint.clone());
        let EndpointSource::Direct {
            capabilities: direct_capabilities,
            ..
        } = &mut Arc::get_mut(&mut direct.inner).unwrap().source
        else {
            unreachable!("fixture uses a direct endpoint")
        };
        *direct_capabilities = Some(capabilities.clone());
        let discovered_instance = ServiceInstance::new(
            InstanceId::new("discovered").unwrap(),
            endpoint,
            capabilities,
            ServiceWeight::default(),
        );
        assert_eq!(
            direct_capabilities.as_ref().unwrap(),
            discovered_instance.capabilities()
        );
        let (_publisher, discovered) = discovered_client(&runtime, vec![discovered_instance]);

        let mut direct_call = Call::new();
        direct_call
            .headers_mut()
            .insert("x-business-header", HeaderValue::from_static("same-value"));
        let discovery_call = direct_call.clone();

        assert_eq!(
            direct
                .invoke::<Value, _>(MethodId::new(0), direct_call, empty_arguments)
                .await
                .unwrap()
                .into_body(),
            Value::Null
        );
        assert_eq!(
            discovered
                .invoke::<Value, _>(MethodId::new(0), discovery_call, empty_arguments)
                .await
                .unwrap()
                .into_body(),
            Value::Null
        );

        let direct_request = captured.recv().await.unwrap();
        let discovery_request = captured.recv().await.unwrap();
        assert_eq!(direct_request, discovery_request);
        for control in [REQUEST_ID, TIMEOUT_MS, ATTEMPT, SERVICE_GROUP] {
            assert!(!direct_request.headers.contains_key(control));
        }
        assert_eq!(
            direct_request.headers.get("x-business-header").unwrap(),
            "same-value"
        );

        fixture.await.unwrap();
        drop((direct, discovered));
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn https_auto_attempt_context_has_no_version_before_alpn() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint: ServiceEndpoint = format!("https://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
        });
        let config = resilience_config(
            Duration::from_secs(1),
            RetryConfig::builder().max_attempts(1).build().unwrap(),
        );
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let observed = Arc::new(AtomicUsize::new(0));
        let mut client = direct_client(&runtime, endpoint.clone());
        let inner = Arc::get_mut(&mut client.inner).unwrap();
        inner.source = EndpointSource::Direct {
            endpoint,
            capabilities: None,
        };
        inner.attempt_interceptor = Arc::from([erase_interceptor(ObserveAutoHttpVersion {
            calls: observed.clone(),
        })]);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("the fixture closes before TLS can negotiate ALPN");
        assert_eq!(error.attempts(), 1);
        assert_eq!(observed.load(Ordering::SeqCst), 1);

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
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
            &HttpBindingId::default(),
            EndpointBreakerSource::Discovery,
            first_endpoint.as_str(),
        );
        let second_breaker = runtime.inner.endpoint_breaker(
            resilience_service(),
            &HttpBindingId::default(),
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
                attempts_started: Arc::new(AtomicU8::new(0)),
            };
            let service = resilience_service();
            let context = Context::new(ContextParts {
                side: Side::Client,
                stage: InterceptionStage::ClientAttempt,
                request_id: "selection-test".to_owned(),
                binding_id: HttpBindingId::default(),
                http_version: Some(http::Version::HTTP_11),
                interface: service,
                method: service.method(MethodId::new(0)).unwrap(),
                deadline: Deadline::after(Duration::from_secs(1)),
                attempt: std::num::NonZeroU8::new(1),
                endpoint: None,
                headers: HeaderMap::new(),
                extensions: http::Extensions::new(),
                arguments: Some(Arguments::new()),
                response_limit: runtime.inner.config.admission().max_response_body_bytes(),
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
            Arc::from([Arc::new(PanickingRouter) as Arc<dyn InstanceRouter>]);
        let error = router_client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .unwrap_err();
        assert_eq!(error.code().as_str(), "router_panic");

        let mut load_balancer_client = direct_client(&runtime, endpoint);
        Arc::get_mut(&mut load_balancer_client.inner)
            .unwrap()
            .load_balancer = Arc::new(PanickingLoadBalancer);
        let error = load_balancer_client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .unwrap_err();
        assert_eq!(error.code().as_str(), "load_balancer_panic");

        drop(router_client);
        drop(load_balancer_client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn router_cannot_inject_an_instance_before_network_io() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint: ServiceEndpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let runtime = ClientRuntime::builder().build().unwrap();
        let mut client = direct_client(&runtime, endpoint);
        Arc::get_mut(&mut client.inner).unwrap().routers =
            Arc::from([Arc::new(InjectingRouter) as Arc<dyn InstanceRouter>]);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("a router cannot inject an instance outside its input snapshot");
        assert_eq!(error.code().as_str(), "invalid_router_output");
        assert_eq!(error.attempts(), 0);
        assert!(error.request_id().is_some());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "router output validation must complete before opening a connection"
        );

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn empty_router_output_returns_no_instances_before_network_io() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint: ServiceEndpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let runtime = ClientRuntime::builder().build().unwrap();
        let mut client = direct_client(&runtime, endpoint);
        Arc::get_mut(&mut client.inner).unwrap().routers =
            Arc::from([Arc::new(EmptyRouter) as Arc<dyn InstanceRouter>]);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("an empty routed snapshot has no selectable instance");
        assert_eq!(error.code().as_str(), "no_instances");
        assert_eq!(error.attempts(), 0);
        assert!(error.request_id().is_some());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "empty router output must be rejected before opening a connection"
        );

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn ready_empty_discovery_snapshot_returns_no_instances_before_network_io() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint: ServiceEndpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let runtime = ClientRuntime::builder().build().unwrap();
        let (publisher, client) = discovered_client(&runtime, vec![instance("previous", endpoint)]);
        let empty = publisher.publish_ready(Vec::new()).unwrap();
        assert_eq!(empty.state(), DirectoryState::Ready);
        assert!(empty.instances().is_empty());

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("a Ready discovery snapshot may still contain no instances");
        assert_eq!(error.code().as_str(), "no_instances");
        assert_eq!(error.attempts(), 0);
        assert!(error.request_id().is_some());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "an empty discovery snapshot must be rejected before opening a connection"
        );

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn argument_serialization_panic_is_isolated_before_network_io() {
        let endpoint: ServiceEndpoint = "http://127.0.0.1:1".parse().unwrap();
        let runtime = ClientRuntime::builder().build().unwrap();
        let client = direct_client(&runtime, endpoint);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), || {
                panic!("private argument serializer panic");
            })
            .await
            .unwrap_err();
        assert_eq!(error.code().as_str(), "serialization_panic");
        assert_eq!(error.attempts(), 0);
        assert!(error.request_id().is_some());
        assert!(!error.message().contains("private argument serializer"));

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn request_encoder_panic_is_local_and_does_not_sample_breakers() {
        let endpoint: ServiceEndpoint = "http://127.0.0.1:1".parse().unwrap();
        let binding_id = HttpBindingId::new("panic-request-v1").unwrap();
        let runtime = ClientRuntime::builder()
            .http_binding(
                binding_id.clone(),
                PanickingRequestEncoder,
                crate::wire::JsonCodec,
                crate::wire::JsonCodec,
            )
            .build()
            .unwrap();
        let client = direct_client_with_binding(&runtime, endpoint.clone(), binding_id.clone());

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("the request encoder panic is contained");
        assert_codec_panic(&error, 0);
        assert_binding_breakers_unsampled(&runtime, &endpoint, &binding_id);

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn custom_encoder_control_header_is_rejected_before_network_io() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint: ServiceEndpoint = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let binding_id = HttpBindingId::new("reserved-header-v1").unwrap();
        let metrics = AttemptMetrics::default();
        let runtime = ClientRuntime::builder()
            .http_binding(
                binding_id.clone(),
                ReservedControlHeaderEncoder,
                crate::wire::JsonCodec,
                crate::wire::JsonCodec,
            )
            .metrics(metrics.clone())
            .build()
            .unwrap();
        let client = direct_client_with_binding(&runtime, endpoint.clone(), binding_id.clone());

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("runtime control headers remain owned by the runtime");
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.origin(), ErrorOrigin::Local);
        assert_eq!(error.category(), ErrorCategory::InvalidArgument);
        assert_eq!(error.code().as_str(), "header_binding_conflict");
        assert_eq!(error.attempts(), 0);
        assert!(error.request_id().is_some());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err(),
            "request validation must complete before opening a connection"
        );
        assert!(
            metrics
                .finished
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        );
        assert_binding_breakers_unsampled(&runtime, &endpoint, &binding_id);

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn response_decoder_panic_is_local_and_does_not_sample_breakers() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "response-codec-panic",
            StatusCode::OK,
            Bytes::from_static(br#""ignored""#),
            None,
            captured_tx,
        )
        .await;
        let binding_id = HttpBindingId::new("panic-response-v1").unwrap();
        let runtime = ClientRuntime::builder()
            .http_binding(
                binding_id.clone(),
                crate::wire::JsonCodec,
                PanickingResponseDecoder,
                crate::wire::JsonCodec,
            )
            .build()
            .unwrap();
        let client = direct_client_with_binding(&runtime, endpoint.clone(), binding_id.clone());

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("the response decoder panic is contained");
        assert_codec_panic(&error, 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert!(captured_rx.try_recv().is_err());
        assert_binding_breakers_unsampled(&runtime, &endpoint, &binding_id);

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn response_decoder_cannot_expand_body_past_runtime_limit() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "response-codec-expansion",
            StatusCode::OK,
            Bytes::from_static(br#""ok""#),
            None,
            captured_tx,
        )
        .await;
        let binding_id = HttpBindingId::new("expanding-response-v1").unwrap();
        let admission = ClientAdmissionConfig::builder()
            .max_response_body_bytes(32)
            .max_inflight_response_body_bytes(64)
            .build()
            .unwrap();
        let config = ClientConfig::builder()
            .admission(admission)
            .build()
            .unwrap();
        let runtime = ClientRuntime::builder()
            .config(config)
            .http_binding(
                binding_id.clone(),
                crate::wire::JsonCodec,
                ExpandingResponseDecoder,
                crate::wire::JsonCodec,
            )
            .build()
            .unwrap();
        let client = direct_client_with_binding(&runtime, endpoint.clone(), binding_id.clone());

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("the decoded body exceeds the runtime response limit");
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.origin(), ErrorOrigin::Local);
        assert_eq!(error.category(), ErrorCategory::PayloadTooLarge);
        assert_eq!(error.code().as_str(), "response_too_large");
        assert_eq!(error.attempts(), 1);
        assert!(error.request_id().is_some());
        assert_eq!(runtime.inner.response_budget.used(), 0);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert!(captured_rx.try_recv().is_err());
        assert_binding_breakers_unsampled(&runtime, &endpoint, &binding_id);

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn error_decoder_panic_is_local_and_does_not_sample_breakers() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "error-codec-panic",
            StatusCode::BAD_REQUEST,
            Bytes::from_static(b"not-a-problem"),
            None,
            captured_tx,
        )
        .await;
        let binding_id = HttpBindingId::new("panic-error-v1").unwrap();
        let runtime = ClientRuntime::builder()
            .http_binding(
                binding_id.clone(),
                crate::wire::JsonCodec,
                crate::wire::JsonCodec,
                PanickingErrorDecoder,
            )
            .build()
            .unwrap();
        let client = direct_client_with_binding(&runtime, endpoint.clone(), binding_id.clone());

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("the error decoder panic is contained");
        assert_codec_panic(&error, 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert!(captured_rx.try_recv().is_err());
        assert_binding_breakers_unsampled(&runtime, &endpoint, &binding_id);

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn head_error_calls_custom_decoder_without_buffering_the_response_body() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_head_error_endpoint(captured_tx).await;
        let binding_id = HttpBindingId::new("custom-head-error-v1").unwrap();
        let observation = Arc::new(Mutex::new(None));
        let admission = ClientAdmissionConfig::builder()
            .max_response_body_bytes(1)
            .max_inflight_response_body_bytes(1)
            .build()
            .unwrap();
        let config = ClientConfig::builder()
            .admission(admission)
            .retry(RetryConfig::builder().max_attempts(1).build().unwrap())
            .build()
            .unwrap();
        let runtime = ClientRuntime::builder()
            .config(config)
            .http_binding(
                binding_id.clone(),
                crate::wire::JsonCodec,
                crate::wire::JsonCodec,
                InspectingHeadErrorDecoder {
                    observation: observation.clone(),
                },
            )
            .build()
            .unwrap();
        let client =
            direct_head_client_with_binding(&runtime, endpoint.clone(), binding_id.clone());

        let error = client
            .invoke::<(), _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("the custom decoder maps the HEAD failure");
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.origin(), ErrorOrigin::Remote);
        assert_eq!(error.category(), ErrorCategory::Unavailable);
        assert_eq!(error.code().as_str(), "custom_head_unavailable");
        assert_eq!(error.attempts(), 1);
        assert_eq!(error.headers().get("x-codec-visible").unwrap(), "yes");
        assert!(!error.headers().contains_key(CONTENT_LENGTH));

        assert_eq!(
            observation
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take(),
            Some(HeadErrorObservation {
                method: Method::HEAD,
                status: StatusCode::SERVICE_UNAVAILABLE,
                content_length: "1048576".to_owned(),
                body: Bytes::new(),
            })
        );
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert!(captured_rx.try_recv().is_err());

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn head_error_decoder_panic_is_local_and_does_not_sample_breakers() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_head_error_endpoint(captured_tx).await;
        let binding_id = HttpBindingId::new("panic-head-error-v1").unwrap();
        let runtime = ClientRuntime::builder()
            .http_binding(
                binding_id.clone(),
                crate::wire::JsonCodec,
                crate::wire::JsonCodec,
                PanickingErrorDecoder,
            )
            .build()
            .unwrap();
        let client =
            direct_head_client_with_binding(&runtime, endpoint.clone(), binding_id.clone());

        let error = client
            .invoke::<(), _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("the HEAD error decoder panic is contained");
        assert_codec_panic(&error, 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert!(captured_rx.try_recv().is_err());
        assert_service_binding_breakers_unsampled(&runtime, head_service(), &endpoint, &binding_id);

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn custom_error_decoder_sees_semantic_headers_and_returns_a_remote_failure() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_decoder_header_endpoint(captured_tx).await;
        let binding_id = HttpBindingId::new("custom-error-v1").unwrap();
        let seen_headers = Arc::new(Mutex::new(None));
        let runtime = ClientRuntime::builder()
            .config(resilience_config(
                Duration::from_secs(1),
                RetryConfig::builder().max_attempts(1).build().unwrap(),
            ))
            .http_binding(
                binding_id.clone(),
                crate::wire::JsonCodec,
                crate::wire::JsonCodec,
                InspectingErrorDecoder {
                    headers: seen_headers.clone(),
                },
            )
            .build()
            .unwrap();
        let client = direct_client_with_binding(&runtime, endpoint.clone(), binding_id.clone());

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("the custom decoder maps the 503 response to an invocation error");
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.origin(), ErrorOrigin::Remote);
        assert_eq!(error.category(), ErrorCategory::Unavailable);
        assert_eq!(error.code().as_str(), "custom_remote_unavailable");
        assert_eq!(error.retry_hint(), RetryHint::Retryable);
        assert_eq!(error.attempts(), 1);
        assert_eq!(error.headers().get("x-codec-visible").unwrap(), "yes");
        assert_eq!(error.headers().get(RETRY_AFTER).unwrap(), "0");
        assert!(!error.headers().contains_key(CONTENT_TYPE));
        assert!(!error.headers().contains_key(REQUEST_ID));

        let headers = seen_headers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .expect("the custom decoder observed one response");
        assert_eq!(headers.get(CONTENT_TYPE).unwrap(), PROBLEM_CONTENT_TYPE);
        assert_eq!(headers.get(RETRY_AFTER).unwrap(), "0");
        assert_eq!(headers.get("x-codec-visible").unwrap(), "yes");
        for control in [REQUEST_ID, ATTEMPT, TIMEOUT_MS, SERVICE_GROUP] {
            assert!(!headers.contains_key(control));
        }

        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert!(captured_rx.try_recv().is_err());
        let endpoint_breaker = runtime
            .inner
            .endpoint_breaker(
                resilience_service(),
                &binding_id,
                EndpointBreakerSource::Direct,
                endpoint.as_str(),
            )
            .snapshot();
        assert_eq!(endpoint_breaker.state, BreakerState::Open);
        let service_breaker = runtime
            .inner
            .service_breaker(resilience_service(), &binding_id)
            .snapshot();
        assert_eq!(service_breaker.state, BreakerState::Open);

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn non_success_body_uses_the_configured_response_limit_before_status_fallback() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "large-error-body",
            StatusCode::SERVICE_UNAVAILABLE,
            Bytes::from(vec![b'<'; 5 * 1024]),
            None,
            captured_tx,
        )
        .await;
        let admission = ClientAdmissionConfig::builder()
            .max_response_body_bytes(8 * 1024)
            .max_inflight_response_body_bytes(8 * 1024)
            .build()
            .unwrap();
        let config = ClientConfig::builder()
            .admission(admission)
            .retry(RetryConfig::builder().max_attempts(1).build().unwrap())
            .build()
            .unwrap();
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let client = direct_client(&runtime, endpoint);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("an invalid Problem body falls back to its HTTP status");
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.origin(), ErrorOrigin::Remote);
        assert_eq!(error.category(), ErrorCategory::Unavailable);
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.code().as_str(), "remote_http_error");
        assert_eq!(error.retry_hint(), RetryHint::Retryable);
        assert_eq!(error.attempts(), 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert!(captured_rx.try_recv().is_err());

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unsupported_response_http_version_is_a_remote_protocol_failure() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_http10_response_endpoint(captured_tx).await;
        let config = ClientConfig::builder()
            .retry(RetryConfig::builder().max_attempts(1).build().unwrap())
            .build()
            .unwrap();
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let client = direct_client(&runtime, endpoint);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("HTTP/1.0 responses are outside the endpoint capability contract");
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.origin(), ErrorOrigin::Remote);
        assert_eq!(error.category(), ErrorCategory::DataLoss);
        assert_eq!(error.code().as_str(), "invalid_http_version");
        assert_eq!(error.retry_hint(), RetryHint::Never);
        assert_eq!(error.attempts(), 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert!(captured_rx.try_recv().is_err());

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn https_auto_attempt_metric_omits_version_when_alpn_never_completes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint: ServiceEndpoint = format!("https://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let fixture = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            drop(stream);
        });
        let config = resilience_config(
            Duration::from_secs(1),
            RetryConfig::builder().max_attempts(1).build().unwrap(),
        );
        let metrics = AttemptVersionMetrics::default();
        let runtime = ClientRuntime::builder()
            .config(config)
            .metrics(metrics.clone())
            .build()
            .unwrap();
        let mut client = direct_client(&runtime, endpoint.clone());
        Arc::get_mut(&mut client.inner).unwrap().source = EndpointSource::Direct {
            endpoint,
            capabilities: None,
        };

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("the peer closes before TLS and ALPN complete");
        assert_eq!(error.attempts(), 1);
        fixture.await.unwrap();
        assert_eq!(
            metrics
                .finished
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_slice(),
            &[None]
        );

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn attempt_interceptor_short_circuits_keep_zero_physical_attempts() {
        let endpoint: ServiceEndpoint = "http://127.0.0.1:1".parse().unwrap();
        let metrics = AttemptMetrics::default();
        let runtime = ClientRuntime::builder()
            .metrics(metrics.clone())
            .build()
            .unwrap();
        let mut client = direct_client(&runtime, endpoint);
        Arc::get_mut(&mut client.inner).unwrap().attempt_interceptor =
            Arc::from([erase_interceptor(ShortCircuitAttempt)]);

        let response = client
            .invoke::<String, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect("the attempt interceptor returns a local response");
        assert_eq!(response.body(), "short-circuit");
        assert_eq!(response.attempts(), 0);

        let mut call = Call::new();
        call.headers_mut()
            .insert("x-short-circuit-error", HeaderValue::from_static("true"));
        let error = client
            .invoke::<String, _>(MethodId::new(0), call, empty_arguments)
            .await
            .expect_err("the attempt interceptor rejects before transport");
        assert_eq!(error.code().as_str(), "short_circuit_error");
        assert_eq!(error.attempts(), 0);
        assert!(error.request_id().is_some());
        assert!(
            metrics
                .finished
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        );

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn zero_attempt_short_circuits_preserve_half_open_breaker_progress() {
        let endpoint: ServiceEndpoint = "http://127.0.0.1:1".parse().unwrap();
        let endpoint_key = endpoint.as_str().to_owned();
        let config = resilience_config_with_close_successes(
            Duration::from_secs(2),
            RetryConfig::default(),
            2,
            2,
        );
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let mut client = direct_client(&runtime, endpoint);
        Arc::get_mut(&mut client.inner).unwrap().attempt_interceptor =
            Arc::from([erase_interceptor(ShortCircuitAttempt)]);
        let endpoint_breaker = runtime.inner.endpoint_breaker(
            resilience_service(),
            &HttpBindingId::default(),
            EndpointBreakerSource::Direct,
            &endpoint_key,
        );
        let service_breaker = runtime
            .inner
            .service_breaker(resilience_service(), &HttpBindingId::default());

        for reject in [false, true] {
            endpoint_breaker
                .try_acquire()
                .unwrap()
                .fail(FailureClass::Transport);
            service_breaker
                .try_acquire()
                .unwrap()
                .fail(FailureClass::Transport);
            tokio::time::advance(Duration::from_secs(15)).await;
            endpoint_breaker.try_acquire().unwrap().succeed();
            service_breaker.try_acquire().unwrap().succeed();
            assert_eq!(endpoint_breaker.snapshot().state, BreakerState::HalfOpen);
            assert_eq!(service_breaker.snapshot().state, BreakerState::HalfOpen);

            let mut call = Call::new();
            if reject {
                call.headers_mut()
                    .insert("x-short-circuit-error", HeaderValue::from_static("true"));
            }
            let result = client
                .invoke::<String, _>(MethodId::new(0), call, empty_arguments)
                .await;
            if reject {
                assert_eq!(result.unwrap_err().attempts(), 0);
            } else {
                assert_eq!(result.unwrap().attempts(), 0);
            }
            assert_eq!(endpoint_breaker.snapshot().state, BreakerState::HalfOpen);
            assert_eq!(service_breaker.snapshot().state, BreakerState::HalfOpen);

            endpoint_breaker.try_acquire().unwrap().succeed();
            service_breaker.try_acquire().unwrap().succeed();
            assert_eq!(endpoint_breaker.snapshot().state, BreakerState::Closed);
            assert_eq!(service_breaker.snapshot().state, BreakerState::Closed);
        }

        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_policy_panic_stops_after_the_first_attempt() {
        let problem = ProblemDetails::new(
            "urn:fusen:error:unavailable:fixture_unavailable",
            "Service Unavailable",
            StatusCode::SERVICE_UNAVAILABLE.as_u16(),
            Some("retry later".to_owned()),
            None,
            ErrorCode::new("fixture_unavailable").unwrap(),
            "fixture-request",
            true,
            None,
        );
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
                RetryConfig::builder()
                    .backoff_base(Duration::from_nanos(1))
                    .backoff_cap(Duration::from_nanos(1))
                    .build()
                    .unwrap(),
            ))
            .retry_policy(PanickingRetryPolicy)
            .build()
            .unwrap();
        let client = direct_client(&runtime, endpoint);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
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
    async fn truncated_response_is_a_terminal_protocol_failure() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (broken_endpoint, broken_fixture) = spawn_broken_body_endpoint(captured_tx).await;
        let config = resilience_config(
            Duration::from_secs(2),
            RetryConfig::builder()
                .backoff_base(Duration::from_nanos(1))
                .backoff_cap(Duration::from_nanos(1))
                .build()
                .unwrap(),
        );
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let client = direct_client(&runtime, broken_endpoint.clone());

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("a truncated response is a protocol failure");
        assert_eq!(error.category(), ErrorCategory::DataLoss);
        assert_eq!(error.attempts(), 1);
        let captured = captured_rx.recv().await.unwrap();
        assert_eq!((captured.endpoint, captured.attempt), ("broken", 1));
        assert!(captured_rx.try_recv().is_err());

        let endpoint = runtime
            .inner
            .endpoint_breaker(
                resilience_service(),
                &HttpBindingId::default(),
                EndpointBreakerSource::Direct,
                broken_endpoint.as_str(),
            )
            .snapshot();
        assert_eq!(endpoint.state, BreakerState::Open);
        let service = runtime
            .inner
            .service_breaker(resilience_service(), &HttpBindingId::default())
            .snapshot();
        assert_eq!(service.state, BreakerState::Open);

        broken_fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_selects_from_a_newer_directory_snapshot_after_a_failed_attempt() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (release_failure, failure_release) = oneshot::channel();
        let (old_endpoint, old_fixture) =
            spawn_gated_retryable_endpoint(captured_tx.clone(), failure_release).await;
        let (new_endpoint, new_fixture) = spawn_full_endpoint(
            "new",
            StatusCode::OK,
            Bytes::from_static(br#""new-snapshot""#),
            None,
            captured_tx,
        )
        .await;
        let config = resilience_config(
            Duration::from_secs(2),
            RetryConfig::builder()
                .backoff_base(Duration::from_nanos(1))
                .backoff_cap(Duration::from_nanos(1))
                .build()
                .unwrap(),
        );
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let (publisher, client) =
            discovered_client(&runtime, vec![instance("old", old_endpoint.clone())]);
        let initial_revision = match &client.inner.source {
            EndpointSource::Discovery(directory) => directory.snapshot().revision(),
            EndpointSource::Direct { .. } => unreachable!("fixture uses service discovery"),
        };

        let invocation = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
                    .await
            }
        });
        let first = captured_rx.recv().await.unwrap();
        assert_eq!((first.endpoint, first.attempt), ("old", 1));

        let latest_instances = vec![instance("new", new_endpoint.clone())];
        runtime
            .inner
            .endpoint_breakers
            .replace_discovery(resilience_service().selector(), &latest_instances);
        let latest = publisher.publish_ready(latest_instances).unwrap();
        assert!(latest.revision() > initial_revision);
        assert_eq!(latest.instances()[0].endpoint(), &new_endpoint);
        release_failure.send(()).unwrap();

        let value = invocation.await.unwrap().unwrap().into_body();
        assert_eq!(value, json!("new-snapshot"));
        let second = captured_rx.recv().await.unwrap();
        assert_eq!((second.endpoint, second.attempt), ("new", 2));
        assert_eq!(first.request_id, second.request_id);

        old_fixture.await.unwrap();
        new_fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retry_selection_failure_preserves_the_completed_attempt_count() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (release_failure, failure_release) = oneshot::channel();
        let (endpoint, fixture) =
            spawn_gated_retryable_endpoint(captured_tx, failure_release).await;
        let config = resilience_config(
            Duration::from_secs(2),
            RetryConfig::builder()
                .backoff_base(Duration::from_nanos(1))
                .backoff_cap(Duration::from_nanos(1))
                .build()
                .unwrap(),
        );
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let (publisher, client) = discovered_client(&runtime, vec![instance("old", endpoint)]);

        let invocation = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
                    .await
            }
        });
        let first = captured_rx.recv().await.unwrap();
        assert_eq!((first.endpoint, first.attempt), ("old", 1));

        publisher
            .publish_state(DirectoryState::Unavailable)
            .unwrap();
        release_failure.send(()).unwrap();

        let error = invocation
            .await
            .unwrap()
            .expect_err("the retry cannot select from an unavailable directory");
        assert_eq!(error.code().as_str(), "no_instances");
        assert_eq!(error.attempts(), 1);
        assert_eq!(error.request_id(), Some(first.request_id.as_str()));
        assert!(captured_rx.try_recv().is_err());

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn infeasible_retry_after_stops_without_spending_a_retry_token() {
        let problem = ProblemDetails::new(
            "urn:fusen:error:unavailable:fixture_unavailable",
            "Service Unavailable",
            StatusCode::SERVICE_UNAVAILABLE.as_u16(),
            Some("retry later".to_owned()),
            None,
            ErrorCode::new("fixture_unavailable").unwrap(),
            "fixture-request",
            true,
            None,
        );
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
            RetryConfig::builder()
                .backoff_base(Duration::from_nanos(1))
                .backoff_cap(Duration::from_nanos(1))
                .budget_capacity(1)
                .budget_refill_per_second(1)
                .build()
                .unwrap(),
        );
        let runtime = ClientRuntime::builder().config(config).build().unwrap();
        let client = direct_client(&runtime, endpoint);

        let error = client
            .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("Retry-After does not fit in the logical deadline");
        assert_eq!(error.attempts(), 1);
        let captured = captured_rx.recv().await.unwrap();
        assert_eq!(captured.attempt, 1);
        assert_eq!(
            runtime
                .inner
                .retry_budget(resilience_service(), &HttpBindingId::default())
                .available(),
            1
        );

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remote_application_status_controls_breakers_without_enabling_retry() {
        for (status, category, breaker_failure) in [
            (StatusCode::CONFLICT, ErrorCategory::Conflict, false),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCategory::Internal,
                true,
            ),
        ] {
            let problem = ProblemDetails::new(
                "urn:fusen:error:application:fixture_application",
                "Application Error",
                status.as_u16(),
                Some("application rejected the invocation".to_owned()),
                None,
                ErrorCode::new("fixture_application").unwrap(),
                "fixture-request",
                true,
                None,
            );
            let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
            let (endpoint, fixture) = spawn_full_endpoint(
                "application",
                status,
                Bytes::from(serde_json::to_vec(&problem).unwrap()),
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
            let client = direct_client(&runtime, endpoint);

            let error = client
                .invoke::<Value, _>(MethodId::new(0), Call::new(), empty_arguments)
                .await
                .expect_err("the fixture returns an application error");
            assert_eq!(error.kind(), ErrorKind::Application);
            assert_eq!(error.origin(), ErrorOrigin::Remote);
            assert_eq!(error.category(), category);
            assert!(!error.retry_hint().is_retryable());
            assert_eq!(error.attempts(), 1);
            assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
            assert!(captured_rx.try_recv().is_err());

            let endpoint = runtime
                .inner
                .endpoint_breaker(
                    resilience_service(),
                    &HttpBindingId::default(),
                    EndpointBreakerSource::Direct,
                    &endpoint_key,
                )
                .snapshot();
            let service = runtime
                .inner
                .service_breaker(resilience_service(), &HttpBindingId::default())
                .snapshot();
            if breaker_failure {
                assert_eq!(endpoint.state, BreakerState::Open);
                assert_eq!(service.state, BreakerState::Open);
            } else {
                assert_eq!((endpoint.samples, endpoint.failures), (1, 0));
                assert_eq!((service.samples, service.failures), (1, 0));
            }

            fixture.await.unwrap();
            drop(client);
            runtime.shutdown().await.unwrap();
        }
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
        let metrics = CombinedMetrics::default();
        let runtime = ClientRuntime::builder()
            .metrics(metrics.clone())
            .build()
            .unwrap();
        let endpoint_key = endpoint.as_str().to_owned();
        let client = direct_client(&runtime, endpoint);

        let error = client
            .invoke::<StructuredResult, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("a scalar result cannot decode into the generated object type");
        assert_eq!(error.category(), ErrorCategory::DataLoss);
        assert_eq!(error.code().as_str(), "invalid_result");
        assert_eq!(error.kind(), ErrorKind::Framework);
        assert_eq!(error.origin(), ErrorOrigin::Remote);
        assert_eq!(error.attempts(), 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert_eq!(metrics.invocation.started.load(Ordering::SeqCst), 1);
        assert_eq!(metrics.invocation.succeeded.load(Ordering::SeqCst), 0);
        assert_eq!(metrics.invocation.failed.load(Ordering::SeqCst), 1);
        {
            let attempts = metrics
                .attempt
                .finished
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            assert_eq!(
                attempts.as_slice(),
                &[(1, MetricOutcome::Error, Some("protocol".into()))]
            );
        }
        let service = runtime
            .inner
            .service_breaker(resilience_service(), &HttpBindingId::default())
            .snapshot();
        assert_eq!((service.samples, service.failures), (1, 1));
        let endpoint = runtime
            .inner
            .endpoint_breaker(
                resilience_service(),
                &HttpBindingId::default(),
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
    async fn attempt_interceptor_fallback_preserves_the_observed_failure_class() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (broken_endpoint, broken_fixture) = spawn_broken_body_endpoint(captured_tx).await;
        let metrics = AttemptMetrics::default();
        let runtime = ClientRuntime::builder()
            .config(resilience_config(
                Duration::from_secs(2),
                RetryConfig::default(),
            ))
            .metrics(metrics.clone())
            .build()
            .unwrap();
        let endpoint_key = broken_endpoint.as_str().to_owned();
        let mut client = direct_client(&runtime, broken_endpoint);
        Arc::get_mut(&mut client.inner).unwrap().attempt_interceptor =
            Arc::from([erase_interceptor(RecoverAttemptError)]);

        let response = client
            .invoke::<String, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect("the attempt interceptor returns a valid fallback response");
        assert_eq!(response.body(), "fallback");
        assert_eq!(response.attempts(), 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        assert!(captured_rx.try_recv().is_err());

        {
            let finished = metrics
                .finished
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            assert_eq!(finished.len(), 1);
            assert_eq!(finished[0].0, 1);
            assert_eq!(finished[0].1, MetricOutcome::Error);
            assert_eq!(finished[0].2.as_deref(), Some("protocol"));
        }

        let endpoint = runtime
            .inner
            .endpoint_breaker(
                resilience_service(),
                &HttpBindingId::default(),
                EndpointBreakerSource::Direct,
                &endpoint_key,
            )
            .snapshot();
        assert!(
            endpoint.state == BreakerState::Open || (endpoint.samples, endpoint.failures) == (1, 1)
        );
        let service = runtime
            .inner
            .service_breaker(resilience_service(), &HttpBindingId::default())
            .snapshot();
        assert!(
            service.state == BreakerState::Open || (service.samples, service.failures) == (1, 1)
        );

        broken_fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn interceptor_replacement_preserves_the_physical_attempt_count() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "interceptor-replacement-attempts",
            StatusCode::OK,
            Bytes::from_static(br#""remote""#),
            None,
            captured_tx,
        )
        .await;
        let metrics = AttemptMetrics::default();
        let runtime = ClientRuntime::builder()
            .metrics(metrics.clone())
            .build()
            .unwrap();
        let mut client = direct_client(&runtime, endpoint);
        Arc::get_mut(&mut client.inner).unwrap().interceptor =
            Arc::from([erase_interceptor(ReplaceRemoteResult)]);

        let response = client
            .invoke::<String, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect("the replacement response matches the generated return type");
        assert_eq!(response.body(), "interceptor-result");
        assert_eq!(response.attempts(), 1);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        {
            let attempts = metrics
                .finished
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            assert_eq!(attempts.as_slice(), &[(1, MetricOutcome::Success, None)]);
        }

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn interceptor_replacement_decode_failure_does_not_poison_breakers() {
        let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
        let (endpoint, fixture) = spawn_full_endpoint(
            "interceptor-replacement",
            StatusCode::OK,
            Bytes::from_static(br#"{"_value":"remote"}"#),
            None,
            captured_tx,
        )
        .await;
        let metrics = AttemptMetrics::default();
        let runtime = ClientRuntime::builder()
            .config(resilience_config(
                Duration::from_secs(1),
                RetryConfig::default(),
            ))
            .metrics(metrics.clone())
            .build()
            .unwrap();
        let endpoint_key = endpoint.as_str().to_owned();
        let mut client = direct_client(&runtime, endpoint);
        Arc::get_mut(&mut client.inner).unwrap().interceptor =
            Arc::from([erase_interceptor(ReplaceRemoteResult)]);

        let error = client
            .invoke::<StructuredResult, _>(MethodId::new(0), Call::new(), empty_arguments)
            .await
            .expect_err("interceptor replacement does not match the generated return type");
        assert_eq!(error.category(), ErrorCategory::DataLoss);
        assert_eq!(error.code().as_str(), "invalid_result");
        assert_eq!(error.origin(), ErrorOrigin::Local);
        assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
        {
            let attempts = metrics
                .finished
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            assert_eq!(attempts.as_slice(), &[(1, MetricOutcome::Success, None)]);
        }

        let endpoint = runtime
            .inner
            .endpoint_breaker(
                resilience_service(),
                &HttpBindingId::default(),
                EndpointBreakerSource::Direct,
                &endpoint_key,
            )
            .snapshot();
        assert_eq!((endpoint.samples, endpoint.failures), (1, 0));
        let service = runtime
            .inner
            .service_breaker(resilience_service(), &HttpBindingId::default())
            .snapshot();
        assert_eq!((service.samples, service.failures), (0, 0));

        fixture.await.unwrap();
        drop(client);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn transformed_remote_body_decode_failure_does_not_poison_breakers() {
        for (name, interceptor) in [
            (
                "body-mut-replacement",
                erase_interceptor(MutateRemoteResult),
            ),
            ("map-replacement", erase_interceptor(MapRemoteResult)),
        ] {
            let (captured_tx, mut captured_rx) = mpsc::unbounded_channel();
            let (endpoint, fixture) = spawn_full_endpoint(
                name,
                StatusCode::OK,
                Bytes::from_static(br#"{"_value":"remote"}"#),
                None,
                captured_tx,
            )
            .await;
            let metrics = AttemptMetrics::default();
            let runtime = ClientRuntime::builder()
                .config(resilience_config(
                    Duration::from_secs(1),
                    RetryConfig::default(),
                ))
                .metrics(metrics.clone())
                .build()
                .unwrap();
            let endpoint_key = endpoint.as_str().to_owned();
            let mut client = direct_client(&runtime, endpoint);
            Arc::get_mut(&mut client.inner).unwrap().interceptor = Arc::from([interceptor]);

            let error = client
                .invoke::<StructuredResult, _>(MethodId::new(0), Call::new(), empty_arguments)
                .await
                .expect_err("the local replacement does not match the generated return type");
            assert_eq!(error.category(), ErrorCategory::DataLoss);
            assert_eq!(error.code().as_str(), "invalid_result");
            assert_eq!(error.origin(), ErrorOrigin::Local);
            assert_eq!(error.attempts(), 1);
            assert_eq!(captured_rx.recv().await.unwrap().attempt, 1);
            assert_eq!(
                metrics
                    .finished
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_slice(),
                &[(1, MetricOutcome::Success, None)]
            );

            let endpoint = runtime
                .inner
                .endpoint_breaker(
                    resilience_service(),
                    &HttpBindingId::default(),
                    EndpointBreakerSource::Direct,
                    &endpoint_key,
                )
                .snapshot();
            assert_eq!((endpoint.samples, endpoint.failures), (1, 0));
            let service = runtime
                .inner
                .service_breaker(resilience_service(), &HttpBindingId::default())
                .snapshot();
            assert_eq!((service.samples, service.failures), (1, 0));

            fixture.await.unwrap();
            drop(client);
            runtime.shutdown().await.unwrap();
        }
    }
}
