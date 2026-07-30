use crate::{
    ConfigValidationError, ConfigValidationErrorKind,
    resilience::breaker::DEFAULT_ENDPOINT_IDLE_EVICTION,
};
use std::time::Duration;

const MIB: usize = 1024 * 1024;

/// Optional bounded admission queue settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueConfig {
    capacity: usize,
    max_wait: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            capacity: 0,
            max_wait: Duration::from_millis(50),
        }
    }
}

impl QueueConfig {
    /// Starts a builder with fail-fast queueing disabled.
    pub fn builder() -> QueueConfigBuilder {
        QueueConfigBuilder(Self::default())
    }

    /// Returns the maximum queued invocations. Zero means disabled.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the maximum queue wait, bounded by the invocation deadline.
    pub const fn max_wait(&self) -> Duration {
        self.max_wait
    }
}

/// Builder for [`QueueConfig`].
#[derive(Clone, Debug)]
pub struct QueueConfigBuilder(QueueConfig);

impl QueueConfigBuilder {
    /// Sets the maximum queued invocations. Zero disables queueing.
    pub const fn capacity(mut self, capacity: usize) -> Self {
        self.0.capacity = capacity;
        self
    }

    /// Sets the maximum queue wait.
    pub const fn max_wait(mut self, max_wait: Duration) -> Self {
        self.0.max_wait = max_wait;
        self
    }

    /// Validates and builds the queue configuration.
    pub fn build(self) -> Result<QueueConfig, ConfigValidationError> {
        validate_queue(&self.0)?;
        Ok(self.0)
    }
}

/// Client concurrency and buffering limits.
#[derive(Clone, Debug)]
pub struct ClientAdmissionConfig {
    max_in_flight: usize,
    max_in_flight_per_endpoint: usize,
    max_request_body_bytes: usize,
    max_response_body_bytes: usize,
    max_inflight_request_body_bytes: usize,
    max_inflight_response_body_bytes: usize,
    queue: QueueConfig,
}

impl Default for ClientAdmissionConfig {
    fn default() -> Self {
        Self {
            max_in_flight: 1024,
            max_in_flight_per_endpoint: 128,
            max_request_body_bytes: 2 * MIB,
            max_response_body_bytes: 2 * MIB,
            max_inflight_request_body_bytes: 64 * MIB,
            max_inflight_response_body_bytes: 64 * MIB,
            queue: QueueConfig::default(),
        }
    }
}

impl ClientAdmissionConfig {
    /// Starts a builder with bounded production defaults.
    pub fn builder() -> ClientAdmissionConfigBuilder {
        ClientAdmissionConfigBuilder(Self::default())
    }

    /// Returns the runtime-wide logical invocation limit.
    pub const fn max_in_flight(&self) -> usize {
        self.max_in_flight
    }

    /// Returns the per-endpoint physical attempt limit.
    pub const fn max_in_flight_per_endpoint(&self) -> usize {
        self.max_in_flight_per_endpoint
    }

    /// Returns the maximum encoded request body size.
    pub const fn max_request_body_bytes(&self) -> usize {
        self.max_request_body_bytes
    }

    /// Returns the maximum decoded response body size.
    pub const fn max_response_body_bytes(&self) -> usize {
        self.max_response_body_bytes
    }

    /// Returns the runtime-wide encoded request byte budget.
    pub const fn max_inflight_request_body_bytes(&self) -> usize {
        self.max_inflight_request_body_bytes
    }

    /// Returns the runtime-wide buffered response byte budget.
    pub const fn max_inflight_response_body_bytes(&self) -> usize {
        self.max_inflight_response_body_bytes
    }

    /// Returns optional queue behavior.
    pub const fn queue(&self) -> &QueueConfig {
        &self.queue
    }
}

/// Builder for [`ClientAdmissionConfig`].
#[derive(Clone, Debug)]
pub struct ClientAdmissionConfigBuilder(ClientAdmissionConfig);

impl ClientAdmissionConfigBuilder {
    /// Sets the runtime-wide logical invocation limit.
    pub const fn max_in_flight(mut self, value: usize) -> Self {
        self.0.max_in_flight = value;
        self
    }

    /// Sets the per-endpoint physical attempt limit.
    pub const fn max_in_flight_per_endpoint(mut self, value: usize) -> Self {
        self.0.max_in_flight_per_endpoint = value;
        self
    }

    /// Sets the maximum encoded request body size.
    pub const fn max_request_body_bytes(mut self, value: usize) -> Self {
        self.0.max_request_body_bytes = value;
        self
    }

    /// Sets the maximum decoded response body size.
    pub const fn max_response_body_bytes(mut self, value: usize) -> Self {
        self.0.max_response_body_bytes = value;
        self
    }

    /// Sets the runtime-wide encoded request byte budget.
    pub const fn max_inflight_request_body_bytes(mut self, value: usize) -> Self {
        self.0.max_inflight_request_body_bytes = value;
        self
    }

    /// Sets the runtime-wide buffered response byte budget.
    pub const fn max_inflight_response_body_bytes(mut self, value: usize) -> Self {
        self.0.max_inflight_response_body_bytes = value;
        self
    }

    /// Replaces optional queue behavior.
    pub fn queue(mut self, value: QueueConfig) -> Self {
        self.0.queue = value;
        self
    }

    /// Validates and builds admission limits.
    pub fn build(self) -> Result<ClientAdmissionConfig, ConfigValidationError> {
        validate_admission(&self.0)?;
        Ok(self.0)
    }
}

/// Registry-backed discovery deadlines and staleness behavior.
#[derive(Clone, Debug)]
pub struct DiscoveryConfig {
    initial_timeout: Duration,
    operation_timeout: Duration,
    close_timeout: Duration,
    max_staleness: Duration,
    reconnect_base: Duration,
    reconnect_cap: Duration,
    max_subscriptions: usize,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            initial_timeout: Duration::from_secs(5),
            operation_timeout: Duration::from_secs(5),
            close_timeout: Duration::from_secs(5),
            max_staleness: Duration::from_secs(30),
            reconnect_base: Duration::from_millis(100),
            reconnect_cap: Duration::from_secs(30),
            max_subscriptions: 1024,
        }
    }
}

impl DiscoveryConfig {
    /// Starts a builder with bounded production defaults.
    pub fn builder() -> DiscoveryConfigBuilder {
        DiscoveryConfigBuilder(Self::default())
    }

    /// Returns the initial directory-ready deadline.
    pub const fn initial_timeout(&self) -> Duration {
        self.initial_timeout
    }

    /// Returns one provider operation timeout.
    pub const fn operation_timeout(&self) -> Duration {
        self.operation_timeout
    }

    /// Returns one subscription close timeout.
    pub const fn close_timeout(&self) -> Duration {
        self.close_timeout
    }

    /// Returns the maximum routable stale duration.
    pub const fn max_staleness(&self) -> Duration {
        self.max_staleness
    }

    /// Returns the reconnect backoff base.
    pub const fn reconnect_base(&self) -> Duration {
        self.reconnect_base
    }

    /// Returns the reconnect backoff cap.
    pub const fn reconnect_cap(&self) -> Duration {
        self.reconnect_cap
    }

    /// Returns the shared subscription limit.
    pub const fn max_subscriptions(&self) -> usize {
        self.max_subscriptions
    }
}

/// Builder for [`DiscoveryConfig`].
#[derive(Clone, Debug)]
pub struct DiscoveryConfigBuilder(DiscoveryConfig);

impl DiscoveryConfigBuilder {
    /// Sets the initial directory-ready deadline.
    pub const fn initial_timeout(mut self, value: Duration) -> Self {
        self.0.initial_timeout = value;
        self
    }

    /// Sets one provider operation timeout.
    pub const fn operation_timeout(mut self, value: Duration) -> Self {
        self.0.operation_timeout = value;
        self
    }

    /// Sets one subscription close timeout.
    pub const fn close_timeout(mut self, value: Duration) -> Self {
        self.0.close_timeout = value;
        self
    }

    /// Sets how long the last ready snapshot remains routable.
    pub const fn max_staleness(mut self, value: Duration) -> Self {
        self.0.max_staleness = value;
        self
    }

    /// Sets the reconnect backoff base.
    pub const fn reconnect_base(mut self, value: Duration) -> Self {
        self.0.reconnect_base = value;
        self
    }

    /// Sets the reconnect backoff cap.
    pub const fn reconnect_cap(mut self, value: Duration) -> Self {
        self.0.reconnect_cap = value;
        self
    }

    /// Sets the maximum number of shared service subscriptions.
    pub const fn max_subscriptions(mut self, value: usize) -> Self {
        self.0.max_subscriptions = value;
        self
    }

    /// Validates and builds discovery settings.
    pub fn build(self) -> Result<DiscoveryConfig, ConfigValidationError> {
        validate_discovery(&self.0)?;
        Ok(self.0)
    }
}

/// Built-in bounded retry settings.
#[derive(Clone, Debug)]
pub struct RetryConfig {
    max_attempts: u8,
    backoff_base: Duration,
    backoff_cap: Duration,
    budget_capacity: u32,
    budget_refill_per_second: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_base: Duration::from_millis(10),
            backoff_cap: Duration::from_millis(200),
            budget_capacity: 100,
            budget_refill_per_second: 10,
        }
    }
}

impl RetryConfig {
    /// Starts a builder with conservative production defaults.
    pub fn builder() -> RetryConfigBuilder {
        RetryConfigBuilder(Self::default())
    }

    /// Returns the hard attempt limit, including the first attempt.
    pub const fn max_attempts(&self) -> u8 {
        self.max_attempts
    }

    /// Returns the full-jitter backoff base.
    pub const fn backoff_base(&self) -> Duration {
        self.backoff_base
    }

    /// Returns the full-jitter backoff cap.
    pub const fn backoff_cap(&self) -> Duration {
        self.backoff_cap
    }

    /// Returns the per-interface retry token capacity.
    pub const fn budget_capacity(&self) -> u32 {
        self.budget_capacity
    }

    /// Returns retry tokens refilled per second.
    pub const fn budget_refill_per_second(&self) -> u32 {
        self.budget_refill_per_second
    }
}

/// Builder for [`RetryConfig`].
#[derive(Clone, Debug)]
pub struct RetryConfigBuilder(RetryConfig);

impl RetryConfigBuilder {
    /// Sets the hard attempt limit, including the first attempt.
    pub const fn max_attempts(mut self, value: u8) -> Self {
        self.0.max_attempts = value;
        self
    }

    /// Sets the full-jitter backoff base.
    pub const fn backoff_base(mut self, value: Duration) -> Self {
        self.0.backoff_base = value;
        self
    }

    /// Sets the full-jitter backoff cap.
    pub const fn backoff_cap(mut self, value: Duration) -> Self {
        self.0.backoff_cap = value;
        self
    }

    /// Sets the per-interface retry token capacity.
    pub const fn budget_capacity(mut self, value: u32) -> Self {
        self.0.budget_capacity = value;
        self
    }

    /// Sets retry tokens refilled per second.
    pub const fn budget_refill_per_second(mut self, value: u32) -> Self {
        self.0.budget_refill_per_second = value;
        self
    }

    /// Validates and builds retry settings.
    pub fn build(self) -> Result<RetryConfig, ConfigValidationError> {
        validate_retry(&self.0)?;
        Ok(self.0)
    }
}

/// One rolling circuit-breaker threshold set.
#[derive(Clone, Debug)]
pub struct BreakerThreshold {
    window: Duration,
    buckets: u8,
    minimum_samples: u32,
    failure_ratio: f64,
    open_duration: Duration,
    half_open_probes: u32,
    close_successes: u32,
}

impl BreakerThreshold {
    fn endpoint_default() -> Self {
        Self {
            window: Duration::from_secs(10),
            buckets: 10,
            minimum_samples: 20,
            failure_ratio: 0.5,
            open_duration: Duration::from_secs(10),
            half_open_probes: 1,
            close_successes: 2,
        }
    }

    fn service_default() -> Self {
        Self {
            window: Duration::from_secs(30),
            buckets: 10,
            minimum_samples: 50,
            failure_ratio: 0.6,
            open_duration: Duration::from_secs(15),
            half_open_probes: 2,
            close_successes: 3,
        }
    }

    /// Starts a builder from endpoint-breaker defaults.
    pub fn endpoint_builder() -> BreakerThresholdBuilder {
        BreakerThresholdBuilder(Self::endpoint_default())
    }

    /// Starts a builder from interface-breaker defaults.
    pub fn service_builder() -> BreakerThresholdBuilder {
        BreakerThresholdBuilder(Self::service_default())
    }

    /// Returns the rolling window.
    pub const fn window(&self) -> Duration {
        self.window
    }

    /// Returns the bucket count.
    pub const fn buckets(&self) -> u8 {
        self.buckets
    }

    /// Returns the minimum sample count.
    pub const fn minimum_samples(&self) -> u32 {
        self.minimum_samples
    }

    /// Returns the failure ratio threshold.
    pub const fn failure_ratio(&self) -> f64 {
        self.failure_ratio
    }

    /// Returns the initial open duration.
    pub const fn open_duration(&self) -> Duration {
        self.open_duration
    }

    /// Returns the half-open concurrency.
    pub const fn half_open_probes(&self) -> u32 {
        self.half_open_probes
    }

    /// Returns consecutive successes required to close.
    pub const fn close_successes(&self) -> u32 {
        self.close_successes
    }
}

/// Builder for [`BreakerThreshold`].
#[derive(Clone, Debug)]
pub struct BreakerThresholdBuilder(BreakerThreshold);

impl BreakerThresholdBuilder {
    /// Sets the rolling window.
    pub const fn window(mut self, value: Duration) -> Self {
        self.0.window = value;
        self
    }

    /// Sets the rolling bucket count.
    pub const fn buckets(mut self, value: u8) -> Self {
        self.0.buckets = value;
        self
    }

    /// Sets the minimum sample count.
    pub const fn minimum_samples(mut self, value: u32) -> Self {
        self.0.minimum_samples = value;
        self
    }

    /// Sets the failure ratio threshold.
    pub const fn failure_ratio(mut self, value: f64) -> Self {
        self.0.failure_ratio = value;
        self
    }

    /// Sets the initial open duration.
    pub const fn open_duration(mut self, value: Duration) -> Self {
        self.0.open_duration = value;
        self
    }

    /// Sets the half-open concurrency.
    pub const fn half_open_probes(mut self, value: u32) -> Self {
        self.0.half_open_probes = value;
        self
    }

    /// Sets consecutive successes required to close.
    pub const fn close_successes(mut self, value: u32) -> Self {
        self.0.close_successes = value;
        self
    }

    /// Validates and builds breaker thresholds.
    pub fn build(self) -> Result<BreakerThreshold, ConfigValidationError> {
        validate_threshold(&self.0, ThresholdScope::Standalone)?;
        Ok(self.0)
    }
}

/// Interface and endpoint circuit-breaker settings.
#[derive(Clone, Debug)]
pub struct CircuitBreakerConfig {
    endpoint: BreakerThreshold,
    service: BreakerThreshold,
    max_open_duration: Duration,
    max_endpoint_entries: usize,
    idle_eviction: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            endpoint: BreakerThreshold::endpoint_default(),
            service: BreakerThreshold::service_default(),
            max_open_duration: Duration::from_secs(120),
            max_endpoint_entries: 10_000,
            idle_eviction: DEFAULT_ENDPOINT_IDLE_EVICTION,
        }
    }
}

impl CircuitBreakerConfig {
    /// Starts a builder with bounded production defaults.
    pub fn builder() -> CircuitBreakerConfigBuilder {
        CircuitBreakerConfigBuilder(Self::default())
    }

    /// Returns endpoint thresholds.
    pub const fn endpoint(&self) -> &BreakerThreshold {
        &self.endpoint
    }

    /// Returns interface-wide thresholds.
    pub const fn service(&self) -> &BreakerThreshold {
        &self.service
    }

    /// Returns the maximum repeated-open interval.
    pub const fn max_open_duration(&self) -> Duration {
        self.max_open_duration
    }

    /// Returns the endpoint breaker entry limit.
    pub const fn max_endpoint_entries(&self) -> usize {
        self.max_endpoint_entries
    }

    /// Returns the endpoint idle eviction interval.
    pub const fn idle_eviction(&self) -> Duration {
        self.idle_eviction
    }
}

/// Builder for [`CircuitBreakerConfig`].
#[derive(Clone, Debug)]
pub struct CircuitBreakerConfigBuilder(CircuitBreakerConfig);

impl CircuitBreakerConfigBuilder {
    /// Replaces endpoint thresholds.
    pub fn endpoint(mut self, value: BreakerThreshold) -> Self {
        self.0.endpoint = value;
        self
    }

    /// Replaces interface-wide thresholds.
    pub fn service(mut self, value: BreakerThreshold) -> Self {
        self.0.service = value;
        self
    }

    /// Sets the maximum repeated-open interval.
    pub const fn max_open_duration(mut self, value: Duration) -> Self {
        self.0.max_open_duration = value;
        self
    }

    /// Sets the endpoint breaker entry limit.
    pub const fn max_endpoint_entries(mut self, value: usize) -> Self {
        self.0.max_endpoint_entries = value;
        self
    }

    /// Sets the endpoint idle eviction interval.
    pub const fn idle_eviction(mut self, value: Duration) -> Self {
        self.0.idle_eviction = value;
        self
    }

    /// Validates and builds circuit-breaker settings.
    pub fn build(self) -> Result<CircuitBreakerConfig, ConfigValidationError> {
        validate_circuit_breaker(&self.0)?;
        Ok(self.0)
    }
}

/// HTTP and HTTPS connection-pool behavior.
#[derive(Clone, Debug)]
pub struct ClientHttpConfig {
    http1_max_idle_per_host: usize,
    pool_idle_timeout: Option<Duration>,
    http2_connections_per_host: usize,
    http2_keep_alive_interval: Option<Duration>,
    http2_keep_alive_timeout: Duration,
}

impl Default for ClientHttpConfig {
    fn default() -> Self {
        Self {
            http1_max_idle_per_host: 128,
            pool_idle_timeout: Some(Duration::from_secs(90)),
            http2_connections_per_host: 1,
            http2_keep_alive_interval: None,
            http2_keep_alive_timeout: Duration::from_secs(20),
        }
    }
}

impl ClientHttpConfig {
    /// Starts a builder with bounded production defaults.
    pub fn builder() -> ClientHttpConfigBuilder {
        ClientHttpConfigBuilder(Self::default())
    }

    /// Returns the HTTP/1 idle connection limit per host.
    pub const fn http1_max_idle_per_host(&self) -> usize {
        self.http1_max_idle_per_host
    }

    /// Returns the shared connection-pool idle timeout.
    pub const fn pool_idle_timeout(&self) -> Option<Duration> {
        self.pool_idle_timeout
    }

    /// Returns HTTP/2 connection shards per endpoint.
    pub const fn http2_connections_per_host(&self) -> usize {
        self.http2_connections_per_host
    }

    /// Returns the HTTP/2 keep-alive interval.
    pub const fn http2_keep_alive_interval(&self) -> Option<Duration> {
        self.http2_keep_alive_interval
    }

    /// Returns the HTTP/2 keep-alive acknowledgement timeout.
    pub const fn http2_keep_alive_timeout(&self) -> Duration {
        self.http2_keep_alive_timeout
    }
}

/// Builder for [`ClientHttpConfig`].
#[derive(Clone, Debug)]
pub struct ClientHttpConfigBuilder(ClientHttpConfig);

impl ClientHttpConfigBuilder {
    /// Sets the HTTP/1 idle connection limit per host.
    pub const fn http1_max_idle_per_host(mut self, value: usize) -> Self {
        self.0.http1_max_idle_per_host = value;
        self
    }

    /// Sets the shared connection-pool idle timeout.
    pub const fn pool_idle_timeout(mut self, value: Option<Duration>) -> Self {
        self.0.pool_idle_timeout = value;
        self
    }

    /// Sets HTTP/2 connection shards per endpoint.
    pub const fn http2_connections_per_host(mut self, value: usize) -> Self {
        self.0.http2_connections_per_host = value;
        self
    }

    /// Sets the HTTP/2 keep-alive interval.
    pub const fn http2_keep_alive_interval(mut self, value: Option<Duration>) -> Self {
        self.0.http2_keep_alive_interval = value;
        self
    }

    /// Sets the HTTP/2 keep-alive acknowledgement timeout.
    pub const fn http2_keep_alive_timeout(mut self, value: Duration) -> Self {
        self.0.http2_keep_alive_timeout = value;
        self
    }

    /// Validates and builds HTTP pool settings.
    pub fn build(self) -> Result<ClientHttpConfig, ConfigValidationError> {
        validate_http(&self.0)?;
        Ok(self.0)
    }
}

/// Immutable client runtime configuration.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    request_timeout: Duration,
    connect_timeout: Duration,
    shutdown_timeout: Duration,
    admission: ClientAdmissionConfig,
    discovery: DiscoveryConfig,
    retry: RetryConfig,
    circuit_breaker: CircuitBreakerConfig,
    http: ClientHttpConfig,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(10),
            connect_timeout: Duration::from_secs(3),
            shutdown_timeout: Duration::from_secs(30),
            admission: ClientAdmissionConfig::default(),
            discovery: DiscoveryConfig::default(),
            retry: RetryConfig::default(),
            circuit_breaker: CircuitBreakerConfig::default(),
            http: ClientHttpConfig::default(),
        }
    }
}

impl ClientConfig {
    /// Starts a validated configuration builder from production defaults.
    pub fn builder() -> ClientConfigBuilder {
        ClientConfigBuilder(Self::default())
    }

    /// Returns the logical invocation timeout.
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// Returns the TCP connection timeout.
    pub const fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// Returns the total client shutdown budget.
    pub const fn shutdown_timeout(&self) -> Duration {
        self.shutdown_timeout
    }

    /// Returns admission and body limits.
    pub const fn admission(&self) -> &ClientAdmissionConfig {
        &self.admission
    }

    /// Returns discovery settings.
    pub const fn discovery(&self) -> &DiscoveryConfig {
        &self.discovery
    }

    /// Returns retry settings.
    pub const fn retry(&self) -> &RetryConfig {
        &self.retry
    }

    /// Returns circuit-breaker settings.
    pub const fn circuit_breaker(&self) -> &CircuitBreakerConfig {
        &self.circuit_breaker
    }

    /// Returns HTTP and HTTPS pool settings.
    pub const fn http(&self) -> &ClientHttpConfig {
        &self.http
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigValidationError> {
        positive_duration(self.request_timeout, "client.request_timeout")?;
        positive_duration(self.connect_timeout, "client.connect_timeout")?;
        positive_duration(self.shutdown_timeout, "client.shutdown_timeout")?;
        validate_admission(&self.admission)?;
        validate_discovery(&self.discovery)?;
        validate_retry(&self.retry)?;
        validate_circuit_breaker(&self.circuit_breaker)?;
        validate_http(&self.http)
    }
}

/// Builder for [`ClientConfig`].
#[derive(Clone, Debug)]
pub struct ClientConfigBuilder(ClientConfig);

impl ClientConfigBuilder {
    /// Sets the end-to-end logical invocation timeout.
    pub const fn request_timeout(mut self, value: Duration) -> Self {
        self.0.request_timeout = value;
        self
    }

    /// Sets the per-attempt TCP connection timeout.
    pub const fn connect_timeout(mut self, value: Duration) -> Self {
        self.0.connect_timeout = value;
        self
    }

    /// Sets the total shutdown budget.
    pub const fn shutdown_timeout(mut self, value: Duration) -> Self {
        self.0.shutdown_timeout = value;
        self
    }

    /// Replaces admission and body limits.
    pub fn admission(mut self, value: ClientAdmissionConfig) -> Self {
        self.0.admission = value;
        self
    }

    /// Replaces discovery settings.
    pub fn discovery(mut self, value: DiscoveryConfig) -> Self {
        self.0.discovery = value;
        self
    }

    /// Replaces retry settings.
    pub fn retry(mut self, value: RetryConfig) -> Self {
        self.0.retry = value;
        self
    }

    /// Replaces circuit-breaker settings.
    pub fn circuit_breaker(mut self, value: CircuitBreakerConfig) -> Self {
        self.0.circuit_breaker = value;
        self
    }

    /// Replaces HTTP and HTTPS pool settings.
    pub fn http(mut self, value: ClientHttpConfig) -> Self {
        self.0.http = value;
        self
    }

    /// Validates and builds the immutable configuration.
    pub fn build(self) -> Result<ClientConfig, ConfigValidationError> {
        self.0.validate()?;
        Ok(self.0)
    }
}

fn validate_queue(config: &QueueConfig) -> Result<(), ConfigValidationError> {
    if config.capacity > 0 && config.max_wait.is_zero() {
        return Err(inconsistent(
            "client.admission.queue.max_wait",
            "must be positive when queue capacity is non-zero",
        ));
    }
    Ok(())
}

fn validate_admission(config: &ClientAdmissionConfig) -> Result<(), ConfigValidationError> {
    positive_usize(config.max_in_flight, "client.admission.max_in_flight")?;
    positive_usize(
        config.max_in_flight_per_endpoint,
        "client.admission.max_in_flight_per_endpoint",
    )?;
    positive_usize(
        config.max_request_body_bytes,
        "client.admission.max_request_body_bytes",
    )?;
    positive_usize(
        config.max_response_body_bytes,
        "client.admission.max_response_body_bytes",
    )?;
    if config.max_inflight_request_body_bytes < config.max_request_body_bytes {
        return Err(inconsistent(
            "client.admission.max_inflight_request_body_bytes",
            "must be at least max_request_body_bytes",
        ));
    }
    if config.max_inflight_response_body_bytes < config.max_response_body_bytes {
        return Err(inconsistent(
            "client.admission.max_inflight_response_body_bytes",
            "must be at least max_response_body_bytes",
        ));
    }
    validate_queue(&config.queue)
}

fn validate_discovery(config: &DiscoveryConfig) -> Result<(), ConfigValidationError> {
    positive_duration(config.initial_timeout, "client.discovery.initial_timeout")?;
    positive_duration(
        config.operation_timeout,
        "client.discovery.operation_timeout",
    )?;
    positive_duration(config.close_timeout, "client.discovery.close_timeout")?;
    positive_duration(config.reconnect_base, "client.discovery.reconnect_base")?;
    positive_duration(config.reconnect_cap, "client.discovery.reconnect_cap")?;
    if config.reconnect_base > config.reconnect_cap {
        return Err(inconsistent(
            "client.discovery.reconnect_base",
            "must not exceed reconnect_cap",
        ));
    }
    positive_usize(
        config.max_subscriptions,
        "client.discovery.max_subscriptions",
    )
}

fn validate_retry(config: &RetryConfig) -> Result<(), ConfigValidationError> {
    if !(1..=3).contains(&config.max_attempts) {
        return Err(out_of_range(
            "client.retry.max_attempts",
            "must be between 1 and 3 inclusive",
        ));
    }
    positive_duration(config.backoff_base, "client.retry.backoff_base")?;
    positive_duration(config.backoff_cap, "client.retry.backoff_cap")?;
    if config.backoff_base > config.backoff_cap {
        return Err(inconsistent(
            "client.retry.backoff_base",
            "must not exceed backoff_cap",
        ));
    }
    positive_u32(config.budget_capacity, "client.retry.budget_capacity")?;
    positive_u32(
        config.budget_refill_per_second,
        "client.retry.budget_refill_per_second",
    )
}

#[derive(Clone, Copy)]
enum ThresholdScope {
    Endpoint,
    Service,
    Standalone,
}

fn validate_threshold(
    config: &BreakerThreshold,
    scope: ThresholdScope,
) -> Result<(), ConfigValidationError> {
    let paths = match scope {
        ThresholdScope::Endpoint => [
            "client.circuit_breaker.endpoint.window",
            "client.circuit_breaker.endpoint.buckets",
            "client.circuit_breaker.endpoint.minimum_samples",
            "client.circuit_breaker.endpoint.failure_ratio",
            "client.circuit_breaker.endpoint.open_duration",
            "client.circuit_breaker.endpoint.half_open_probes",
            "client.circuit_breaker.endpoint.close_successes",
        ],
        ThresholdScope::Service => [
            "client.circuit_breaker.service.window",
            "client.circuit_breaker.service.buckets",
            "client.circuit_breaker.service.minimum_samples",
            "client.circuit_breaker.service.failure_ratio",
            "client.circuit_breaker.service.open_duration",
            "client.circuit_breaker.service.half_open_probes",
            "client.circuit_breaker.service.close_successes",
        ],
        ThresholdScope::Standalone => [
            "breaker_threshold.window",
            "breaker_threshold.buckets",
            "breaker_threshold.minimum_samples",
            "breaker_threshold.failure_ratio",
            "breaker_threshold.open_duration",
            "breaker_threshold.half_open_probes",
            "breaker_threshold.close_successes",
        ],
    };
    positive_duration(config.window, paths[0])?;
    if config.buckets == 0 {
        return Err(out_of_range(paths[1], "must be greater than zero"));
    }
    positive_u32(config.minimum_samples, paths[2])?;
    if !config.failure_ratio.is_finite() || !(0.0..=1.0).contains(&config.failure_ratio) {
        return Err(out_of_range(paths[3], "must be finite and between 0 and 1"));
    }
    positive_duration(config.open_duration, paths[4])?;
    positive_u32(config.half_open_probes, paths[5])?;
    positive_u32(config.close_successes, paths[6])
}

fn validate_circuit_breaker(config: &CircuitBreakerConfig) -> Result<(), ConfigValidationError> {
    validate_threshold(&config.endpoint, ThresholdScope::Endpoint)?;
    validate_threshold(&config.service, ThresholdScope::Service)?;
    positive_duration(
        config.max_open_duration,
        "client.circuit_breaker.max_open_duration",
    )?;
    positive_usize(
        config.max_endpoint_entries,
        "client.circuit_breaker.max_endpoint_entries",
    )?;
    positive_duration(config.idle_eviction, "client.circuit_breaker.idle_eviction")
}

fn validate_http(config: &ClientHttpConfig) -> Result<(), ConfigValidationError> {
    if config
        .pool_idle_timeout
        .is_some_and(|value| value.is_zero())
    {
        return Err(out_of_range(
            "client.http.pool_idle_timeout",
            "must be positive when configured",
        ));
    }
    positive_usize(
        config.http2_connections_per_host,
        "client.http.http2_connections_per_host",
    )?;
    if config
        .http2_keep_alive_interval
        .is_some_and(|value| value.is_zero())
    {
        return Err(out_of_range(
            "client.http.http2_keep_alive_interval",
            "must be positive when configured",
        ));
    }
    positive_duration(
        config.http2_keep_alive_timeout,
        "client.http.http2_keep_alive_timeout",
    )
}

fn positive_duration(
    value: Duration,
    field_path: &'static str,
) -> Result<(), ConfigValidationError> {
    if value.is_zero() {
        Err(out_of_range(field_path, "must be greater than zero"))
    } else {
        Ok(())
    }
}

fn positive_usize(value: usize, field_path: &'static str) -> Result<(), ConfigValidationError> {
    if value == 0 {
        Err(out_of_range(field_path, "must be greater than zero"))
    } else {
        Ok(())
    }
}

fn positive_u32(value: u32, field_path: &'static str) -> Result<(), ConfigValidationError> {
    if value == 0 {
        Err(out_of_range(field_path, "must be greater than zero"))
    } else {
        Ok(())
    }
}

const fn out_of_range(field_path: &'static str, reason: &'static str) -> ConfigValidationError {
    ConfigValidationError::new(ConfigValidationErrorKind::OutOfRange, field_path, reason)
}

const fn inconsistent(field_path: &'static str, reason: &'static str) -> ConfigValidationError {
    ConfigValidationError::new(ConfigValidationErrorKind::Inconsistent, field_path, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_getters_match_the_runtime_contract() {
        let config = ClientConfig::default();
        assert_eq!(config.request_timeout(), Duration::from_secs(10));
        assert_eq!(config.admission().max_in_flight(), 1024);
        assert_eq!(config.admission().queue().capacity(), 0);
        assert_eq!(
            config.discovery().reconnect_base(),
            Duration::from_millis(100)
        );
        assert_eq!(config.retry().max_attempts(), 3);
        assert_eq!(config.retry().budget_capacity(), 100);
        assert_eq!(config.circuit_breaker().endpoint().minimum_samples(), 20);
        assert_eq!(config.circuit_breaker().service().minimum_samples(), 50);
        assert_eq!(config.http().http2_connections_per_host(), 1);
    }

    #[test]
    fn validation_reports_stable_kind_path_and_reason() {
        let retry = RetryConfig::builder().max_attempts(4).build().unwrap_err();
        assert_eq!(retry.kind(), ConfigValidationErrorKind::OutOfRange);
        assert_eq!(retry.field_path(), "client.retry.max_attempts");
        assert_eq!(retry.reason(), "must be between 1 and 3 inclusive");

        let admission = ClientAdmissionConfig::builder()
            .max_request_body_bytes(1025)
            .max_inflight_request_body_bytes(1024)
            .build()
            .unwrap_err();
        assert_eq!(admission.kind(), ConfigValidationErrorKind::Inconsistent);
        assert_eq!(
            admission.field_path(),
            "client.admission.max_inflight_request_body_bytes"
        );
    }

    #[test]
    fn independent_builders_accept_cross_field_boundaries() {
        let retry = RetryConfig::builder()
            .backoff_base(Duration::from_millis(10))
            .backoff_cap(Duration::from_millis(10))
            .build()
            .unwrap();
        let queue = QueueConfig::builder()
            .capacity(0)
            .max_wait(Duration::ZERO)
            .build()
            .unwrap();
        let admission = ClientAdmissionConfig::builder()
            .queue(queue)
            .build()
            .unwrap();
        ClientConfig::builder()
            .retry(retry)
            .admission(admission)
            .build()
            .unwrap();
    }
}
