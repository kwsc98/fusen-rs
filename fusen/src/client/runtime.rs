use super::{
    config::ClientConfig,
    endpoint_breakers::{EndpointBreakerSource, EndpointBreakers},
    subscription::SubscriptionManager,
    transport::HttpTransport,
};
use crate::{
    ClientError, ClientErrorKind, ErrorDecoder, Interceptor, RequestEncoder, ResponseDecoder,
    RetryPolicy,
    interceptor::erase_interceptor,
    resilience::{
        breaker::{BreakerConfig, BreakerPhase, CircuitBreaker},
        retry::{RetryBudget, StandardRetryPolicy},
    },
    runtime::{admission::AdmissionGate, budget::ByteBudget, metrics::SafeMetrics},
    wire::JsonCodec,
};
use fusen_contract::{HttpBindingId, ServiceDescriptor};
use fusen_observability::{
    CircuitState, CircuitStateChangedEvent, MetricEvent, MetricOutcome, MetricsRecorder,
    ShutdownFinishedEvent,
};
use fusen_register::Registry;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Instant as StdInstant,
};
use tokio::sync::{Semaphore, watch};
use tokio_util::sync::CancellationToken;

pub(crate) const CLIENT_RUNNING: u8 = 0;
const CLIENT_DRAINING: u8 = 1;
const CLIENT_CLOSED: u8 = 2;

/// Observable client runtime lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ClientState {
    /// New logical invocations are admitted.
    Running,
    /// Admission is closed and existing work is draining.
    Draining,
    /// Shutdown reached its shared terminal result.
    Closed,
}

/// Shared client runtime for discovery, interceptor, pools, and resilience state.
#[derive(Clone)]
pub struct ClientRuntime {
    pub(crate) inner: Arc<ClientRuntimeInner>,
}

/// Builder for [`ClientRuntime`].
pub struct ClientRuntimeBuilder {
    config: ClientConfig,
    registry: Option<Arc<dyn Registry>>,
    interceptor: Vec<Arc<dyn Interceptor>>,
    attempt_interceptor: Vec<Arc<dyn Interceptor>>,
    metrics: Option<Arc<dyn MetricsRecorder>>,
    retry_policy: Arc<dyn RetryPolicy>,
    http_bindings: Vec<(HttpBindingId, Arc<ClientHttpBinding>)>,
}

impl ClientRuntime {
    /// Starts a runtime builder with bounded production defaults.
    pub fn builder() -> ClientRuntimeBuilder {
        ClientRuntimeBuilder {
            config: ClientConfig::default(),
            registry: None,
            interceptor: Vec::new(),
            attempt_interceptor: Vec::new(),
            metrics: None,
            retry_policy: Arc::new(StandardRetryPolicy),
            http_bindings: Vec::new(),
        }
    }

    /// Returns the current lifecycle state.
    pub fn state(&self) -> ClientState {
        match self.inner.state.load(Ordering::Acquire) {
            CLIENT_RUNNING => ClientState::Running,
            CLIENT_DRAINING => ClientState::Draining,
            _ => ClientState::Closed,
        }
    }

    /// Requests idempotent background shutdown and waits for its shared terminal result.
    pub async fn shutdown(&self) -> Result<(), ClientError> {
        self.inner.shutdown.cancel();
        let mut completion = self.inner.completion.clone();
        loop {
            if let Some(result) = completion.borrow().clone() {
                return result;
            }
            completion.changed().await.map_err(|error| {
                ClientError::with_source(
                    ClientErrorKind::Shutdown,
                    "client shutdown coordinator ended without a result",
                    error,
                )
            })?;
        }
    }
}

impl ClientRuntimeBuilder {
    /// Replaces immutable client configuration.
    pub fn config(mut self, config: ClientConfig) -> Self {
        self.config = config;
        self
    }

    /// Installs the one registry used by discovery clients.
    pub fn registry<R>(mut self, registry: R) -> Self
    where
        R: Registry + 'static,
    {
        self.registry = Some(Arc::new(registry));
        self
    }

    /// Appends global logical-invocation interceptor.
    pub fn interceptor(mut self, interceptor: impl Interceptor) -> Self {
        self.interceptor.push(erase_interceptor(interceptor));
        self
    }

    /// Appends global interceptor around every physical transport attempt.
    pub fn attempt_interceptor(mut self, interceptor: impl Interceptor) -> Self {
        self.attempt_interceptor
            .push(erase_interceptor(interceptor));
        self
    }

    /// Installs the synchronous, non-blocking metrics recorder.
    pub fn metrics(mut self, recorder: impl MetricsRecorder) -> Self {
        self.metrics = Some(Arc::new(recorder));
        self
    }

    /// Replaces the retry decision extension while retaining runtime hard limits.
    pub fn retry_policy(mut self, policy: impl RetryPolicy) -> Self {
        self.retry_policy = Arc::new(policy);
        self
    }

    /// Registers a complete client-side HTTP binding under one stable identifier.
    pub fn http_binding(
        mut self,
        id: HttpBindingId,
        request_encoder: impl RequestEncoder,
        response_decoder: impl ResponseDecoder,
        error_decoder: impl ErrorDecoder,
    ) -> Self {
        self.http_bindings.push((
            id,
            Arc::new(ClientHttpBinding {
                request_encoder: Arc::new(request_encoder),
                response_decoder: Arc::new(response_decoder),
                error_decoder: Arc::new(error_decoder),
            }),
        ));
        self
    }

    /// Validates and creates all runtime-owned supervisors and HTTP/HTTPS pools.
    pub fn build(self) -> Result<ClientRuntime, ClientError> {
        self.config.validate().map_err(|error| {
            ClientError::with_source(
                ClientErrorKind::Build,
                format!("invalid client configuration at {}", error.field_path()),
                error,
            )
        })?;
        let runtime = tokio::runtime::Handle::try_current().map_err(|error| {
            ClientError::with_source(
                ClientErrorKind::Build,
                "ClientRuntime must be built inside a running Tokio runtime",
                error,
            )
        })?;
        let mut http_bindings = HashMap::new();
        let json = Arc::new(ClientHttpBinding {
            request_encoder: Arc::new(JsonCodec),
            response_decoder: Arc::new(JsonCodec),
            error_decoder: Arc::new(JsonCodec),
        });
        http_bindings.insert(HttpBindingId::default(), json);
        for (id, binding) in self.http_bindings {
            if http_bindings.insert(id.clone(), binding).is_some() {
                return Err(ClientError::from_message(
                    ClientErrorKind::Build,
                    format!("duplicate or reserved HTTP binding {id}"),
                ));
            }
        }
        let config = Arc::new(self.config);
        let admission = AdmissionGate::new(config.admission().max_in_flight());
        let queue_slots = (config.admission().queue().capacity() > 0)
            .then(|| Arc::new(Semaphore::new(config.admission().queue().capacity())));
        let request_budget = ByteBudget::new(config.admission().max_inflight_request_body_bytes());
        let response_budget =
            ByteBudget::new(config.admission().max_inflight_response_body_bytes());
        let transport = Arc::new(Mutex::new(Some(HttpTransport::new(
            config.connect_timeout(),
            config.http(),
        )?)));
        let metrics = SafeMetrics::new(self.metrics);
        let endpoint_threshold = breaker_config(
            config.circuit_breaker().endpoint(),
            config.circuit_breaker().max_open_duration(),
        );
        let endpoint_breakers = EndpointBreakers::new(
            endpoint_threshold,
            config.circuit_breaker().max_endpoint_entries(),
            config.circuit_breaker().idle_eviction(),
        );
        let subscriptions = self.registry.as_ref().map(|registry| {
            SubscriptionManager::new(
                registry.clone(),
                config.discovery().clone(),
                metrics.clone(),
                endpoint_breakers.clone(),
            )
        });
        let shutdown = CancellationToken::new();
        let force_cancel = CancellationToken::new();
        let state = Arc::new(AtomicU8::new(CLIENT_RUNNING));
        let (completion_sender, completion) = watch::channel(None);
        let inner = Arc::new(ClientRuntimeInner {
            config: config.clone(),
            interceptor: Arc::from(self.interceptor),
            attempt_interceptor: Arc::from(self.attempt_interceptor),
            metrics: metrics.clone(),
            retry_policy: self.retry_policy,
            http_bindings,
            admission: admission.clone(),
            queue_slots,
            request_budget,
            response_budget,
            transport: transport.clone(),
            subscriptions: subscriptions.clone(),
            endpoint_breakers,
            service_breakers: Mutex::new(HashMap::new()),
            retry_budgets: Mutex::new(HashMap::new()),
            endpoint_bulkheads: Mutex::new(HashMap::new()),
            shutdown: shutdown.clone(),
            force_cancel: force_cancel.clone(),
            state: state.clone(),
            completion,
        });
        runtime.spawn(client_shutdown_coordinator(ClientShutdown {
            deadline: config.shutdown_timeout(),
            shutdown,
            force_cancel,
            state,
            admission,
            subscriptions,
            transport,
            completion: completion_sender,
            metrics,
        }));
        Ok(ClientRuntime { inner })
    }
}

pub(crate) struct ClientRuntimeInner {
    pub config: Arc<ClientConfig>,
    pub interceptor: Arc<[Arc<dyn Interceptor>]>,
    pub attempt_interceptor: Arc<[Arc<dyn Interceptor>]>,
    pub metrics: SafeMetrics,
    pub retry_policy: Arc<dyn RetryPolicy>,
    pub http_bindings: HashMap<HttpBindingId, Arc<ClientHttpBinding>>,
    pub admission: Arc<AdmissionGate>,
    pub queue_slots: Option<Arc<Semaphore>>,
    pub request_budget: Arc<ByteBudget>,
    pub response_budget: Arc<ByteBudget>,
    pub transport: Arc<Mutex<Option<HttpTransport>>>,
    pub subscriptions: Option<Arc<SubscriptionManager>>,
    pub endpoint_breakers: EndpointBreakers,
    pub service_breakers: Mutex<HashMap<String, Arc<CircuitBreaker>>>,
    pub retry_budgets: Mutex<HashMap<String, Arc<RetryBudget>>>,
    pub endpoint_bulkheads: Mutex<HashMap<String, Arc<Semaphore>>>,
    pub shutdown: CancellationToken,
    pub force_cancel: CancellationToken,
    pub state: Arc<AtomicU8>,
    completion: watch::Receiver<Option<Result<(), ClientError>>>,
}

pub(crate) struct ClientHttpBinding {
    pub request_encoder: Arc<dyn RequestEncoder>,
    pub response_decoder: Arc<dyn ResponseDecoder>,
    pub error_decoder: Arc<dyn ErrorDecoder>,
}

impl ClientRuntimeInner {
    pub(crate) fn transport(&self) -> Result<HttpTransport, ClientError> {
        self.transport
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .ok_or_else(|| {
                ClientError::from_message(
                    ClientErrorKind::Closed,
                    "client connection pool is closed",
                )
            })
    }

    pub(crate) fn service_breaker(
        &self,
        service: &'static ServiceDescriptor,
        binding_id: &HttpBindingId,
    ) -> Arc<CircuitBreaker> {
        let mut breakers = self
            .service_breakers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        breakers
            .entry(binding_key(service, binding_id))
            .or_insert_with(|| {
                let metrics = self.metrics.clone();
                let binding = binding_id.as_str().to_owned();
                let service_id = service.selector().service_id().to_owned();
                CircuitBreaker::observed(
                    breaker_config(
                        self.config.circuit_breaker().service(),
                        self.config.circuit_breaker().max_open_duration(),
                    ),
                    Arc::new(move |phase| {
                        metrics.record(&MetricEvent::CircuitStateChanged(
                            CircuitStateChangedEvent::new(
                                "service",
                                &binding,
                                &service_id,
                                metric_circuit_state(phase),
                            ),
                        ));
                    }),
                )
            })
            .clone()
    }

    pub(crate) fn endpoint_breaker(
        &self,
        service: &'static ServiceDescriptor,
        binding_id: &HttpBindingId,
        source: EndpointBreakerSource,
        endpoint: &str,
    ) -> Arc<CircuitBreaker> {
        let metrics = self.metrics.clone();
        let binding = binding_id.as_str().to_owned();
        let service_id = service.selector().service_id().to_owned();
        self.endpoint_breakers.get_or_insert_observed(
            service.identity(),
            binding_id,
            source,
            endpoint,
            Arc::new(move |phase| {
                metrics.record(&MetricEvent::CircuitStateChanged(
                    CircuitStateChangedEvent::new(
                        "endpoint",
                        &binding,
                        &service_id,
                        metric_circuit_state(phase),
                    ),
                ));
            }),
        )
    }

    pub(crate) fn retry_budget(
        &self,
        service: &'static ServiceDescriptor,
        binding_id: &HttpBindingId,
    ) -> Arc<RetryBudget> {
        let mut budgets = self
            .retry_budgets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        budgets
            .entry(binding_key(service, binding_id))
            .or_insert_with(|| {
                Arc::new(RetryBudget::new(
                    self.config.retry().budget_capacity(),
                    self.config.retry().budget_refill_per_second(),
                ))
            })
            .clone()
    }

    pub(crate) fn endpoint_bulkhead(&self, endpoint: &str) -> Arc<Semaphore> {
        let mut bulkheads = self
            .endpoint_bulkheads
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if bulkheads.len() >= self.config.circuit_breaker().max_endpoint_entries()
            && !bulkheads.contains_key(endpoint)
            && let Some(key) = bulkheads.keys().next().cloned()
        {
            bulkheads.remove(&key);
        }
        bulkheads
            .entry(endpoint.to_owned())
            .or_insert_with(|| {
                Arc::new(Semaphore::new(
                    self.config.admission().max_in_flight_per_endpoint(),
                ))
            })
            .clone()
    }
}

fn binding_key(service: &ServiceDescriptor, binding_id: &HttpBindingId) -> String {
    format!("{}\0{}", service.identity(), binding_id.as_str())
}

const fn metric_circuit_state(phase: BreakerPhase) -> CircuitState {
    match phase {
        BreakerPhase::Closed => CircuitState::Closed,
        BreakerPhase::Open => CircuitState::Open,
        BreakerPhase::HalfOpen => CircuitState::HalfOpen,
    }
}

impl Drop for ClientRuntimeInner {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

fn breaker_config(
    threshold: &super::config::BreakerThreshold,
    max_open_duration: std::time::Duration,
) -> BreakerConfig {
    BreakerConfig::new(
        threshold.window(),
        threshold.buckets(),
        threshold.minimum_samples(),
        threshold.failure_ratio(),
        threshold.open_duration(),
        max_open_duration,
        threshold.half_open_probes(),
        threshold.close_successes(),
    )
}

struct ClientShutdown {
    deadline: std::time::Duration,
    shutdown: CancellationToken,
    force_cancel: CancellationToken,
    state: Arc<AtomicU8>,
    admission: Arc<AdmissionGate>,
    subscriptions: Option<Arc<SubscriptionManager>>,
    transport: Arc<Mutex<Option<HttpTransport>>>,
    completion: watch::Sender<Option<Result<(), ClientError>>>,
    metrics: SafeMetrics,
}

async fn client_shutdown_coordinator(coordinator: ClientShutdown) {
    coordinator.shutdown.cancelled().await;
    let started = StdInstant::now();
    let deadline = tokio::time::Instant::now() + coordinator.deadline;
    coordinator.state.store(CLIENT_DRAINING, Ordering::Release);
    coordinator.admission.begin_draining();
    if let Some(subscriptions) = &coordinator.subscriptions {
        subscriptions.begin_shutdown();
    }
    let work = async {
        let subscriptions = async {
            match &coordinator.subscriptions {
                Some(subscriptions) => subscriptions.closed().await.map_err(|error| {
                    ClientError::with_source(
                        ClientErrorKind::Shutdown,
                        "failed to close discovery subscription",
                        error,
                    )
                }),
                None => Ok(()),
            }
        };
        let invocations = async {
            coordinator.admission.drained().await;
            close_transport(&coordinator.transport);
        };
        let ((), subscriptions) = tokio::join!(invocations, subscriptions);
        subscriptions
    };
    let result = match tokio::time::timeout_at(deadline, work).await {
        Ok(result) => result,
        Err(_) => {
            coordinator.force_cancel.cancel();
            close_transport(&coordinator.transport);
            Err(ClientError::from_message(
                ClientErrorKind::Timeout,
                "client graceful shutdown deadline elapsed",
            ))
        }
    };
    if let Some(subscriptions) = &coordinator.subscriptions {
        subscriptions.finish_shutdown();
    }
    coordinator.admission.close();
    coordinator.state.store(CLIENT_CLOSED, Ordering::Release);
    coordinator
        .metrics
        .record(&MetricEvent::ShutdownFinished(ShutdownFinishedEvent::new(
            "client",
            match &result {
                Ok(()) => MetricOutcome::Success,
                Err(error) if error.kind() == ClientErrorKind::Timeout => MetricOutcome::Timeout,
                Err(_) => MetricOutcome::Error,
            },
            started.elapsed(),
        )));
    coordinator.completion.send_replace(Some(result));
}

fn close_transport(transport: &Mutex<Option<HttpTransport>>) {
    transport
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusen_contract::{
        EndpointCapabilities, InstanceId, ServiceInstance, ServiceSelector, ServiceWeight,
    };
    use fusen_register::{
        RegistrationHandle, RegistrationRequest, SubscriptionHandle, SubscriptionRequest,
        directory::directory,
        error::{RegistryError, RegistryErrorKind, RegistryOperation},
        provider,
    };
    use std::{future::pending, time::Duration};
    use tokio::sync::Notify;

    #[derive(Clone)]
    struct PendingCloseRegistry {
        close_started: Arc<Notify>,
    }

    impl Registry for PendingCloseRegistry {
        fn prepare_registration(
            &self,
            _request: RegistrationRequest,
        ) -> Result<RegistrationHandle, RegistryError> {
            Err(RegistryError::message(
                RegistryOperation::PrepareRegistration,
                RegistryErrorKind::InvalidResource,
                "test registry only supports subscriptions",
            ))
        }

        fn prepare_subscription(
            &self,
            _request: SubscriptionRequest,
        ) -> Result<SubscriptionHandle, RegistryError> {
            let (publisher, directory) = directory();
            let close_started = self.close_started.clone();
            let close_publisher = publisher.clone();
            Ok(provider::subscription(
                directory,
                async move {
                    publisher.publish_ready(vec![ServiceInstance::new(
                        InstanceId::new("shutdown-test").unwrap(),
                        "http://127.0.0.1:8080".parse().unwrap(),
                        EndpointCapabilities::default(),
                        ServiceWeight::default(),
                    )])?;
                    Ok(())
                },
                move || async move {
                    let _publisher = close_publisher;
                    close_started.notify_one();
                    pending::<Result<(), RegistryError>>().await
                },
            ))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn admitted_work_keeps_pool_access_until_it_has_drained() {
        let runtime = ClientRuntime::builder().build().unwrap();
        let admitted = runtime.inner.admission.try_enter().unwrap();
        let shutting_down = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.shutdown().await }
        });

        tokio::task::yield_now().await;
        assert_eq!(runtime.state(), ClientState::Draining);
        assert!(runtime.inner.transport().is_ok());

        drop(admitted);
        shutting_down.await.unwrap().unwrap();
        assert_eq!(runtime.state(), ClientState::Closed);
        assert!(runtime.inner.transport().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_waiter_does_not_cancel_shared_bounded_shutdown() {
        let config = ClientConfig::builder()
            .shutdown_timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let registry = PendingCloseRegistry {
            close_started: Arc::new(Notify::new()),
        };
        let runtime = ClientRuntime::builder()
            .config(config)
            .registry(registry.clone())
            .build()
            .unwrap();
        runtime
            .inner
            .subscriptions
            .as_ref()
            .unwrap()
            .acquire(ServiceSelector::new("shutdown-test", None, None).unwrap())
            .await
            .unwrap();
        let admitted = runtime.inner.admission.try_enter().unwrap();
        let waiter = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.shutdown().await }
        });

        registry.close_started.notified().await;
        assert_eq!(runtime.state(), ClientState::Draining);
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        let first = runtime.shutdown().await.unwrap_err();
        let second = runtime.shutdown().await.unwrap_err();
        assert_eq!(first.kind(), ClientErrorKind::Timeout);
        assert_eq!(second.kind(), ClientErrorKind::Timeout);
        assert_eq!(runtime.state(), ClientState::Closed);
        assert!(runtime.inner.force_cancel.is_cancelled());
        assert!(runtime.inner.transport().is_err());

        drop(admitted);
    }
}
