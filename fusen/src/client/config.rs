use crate::{
    error::{ClientError, ClientErrorKind},
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

impl QueueConfig {
    /// Disables admission queuing and makes overload fail fast.
    pub const fn disabled() -> Self {
        Self {
            capacity: 0,
            max_wait: Duration::from_millis(50),
        }
    }

    /// Enables a bounded queue with a 50 millisecond maximum wait.
    pub fn bounded(capacity: usize) -> Self {
        Self {
            capacity,
            max_wait: Duration::from_millis(50),
        }
    }

    /// Replaces the queue wait limit.
    pub const fn with_max_wait(mut self, max_wait: Duration) -> Self {
        self.max_wait = max_wait;
        self
    }

    /// Returns the maximum queued invocations. Zero means disabled.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the maximum queue wait, still bounded by the invocation deadline.
    pub const fn max_wait(&self) -> Duration {
        self.max_wait
    }
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self::disabled()
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
            queue: QueueConfig::disabled(),
        }
    }
}

impl ClientAdmissionConfig {
    /// Sets the runtime-wide logical invocation limit.
    pub const fn max_in_flight(mut self, value: usize) -> Self {
        self.max_in_flight = value;
        self
    }

    /// Sets the per-endpoint physical attempt limit.
    pub const fn max_in_flight_per_endpoint(mut self, value: usize) -> Self {
        self.max_in_flight_per_endpoint = value;
        self
    }

    /// Sets the maximum encoded request body size.
    pub const fn max_request_body_bytes(mut self, value: usize) -> Self {
        self.max_request_body_bytes = value;
        self
    }

    /// Sets the maximum decoded response body size.
    pub const fn max_response_body_bytes(mut self, value: usize) -> Self {
        self.max_response_body_bytes = value;
        self
    }

    /// Sets the runtime-wide encoded request byte budget.
    pub const fn max_inflight_request_body_bytes(mut self, value: usize) -> Self {
        self.max_inflight_request_body_bytes = value;
        self
    }

    /// Sets the runtime-wide buffered response byte budget.
    pub const fn max_inflight_response_body_bytes(mut self, value: usize) -> Self {
        self.max_inflight_response_body_bytes = value;
        self
    }

    /// Sets optional queue behavior.
    pub fn queue(mut self, value: QueueConfig) -> Self {
        self.queue = value;
        self
    }

    pub(crate) const fn max_in_flight_value(&self) -> usize {
        self.max_in_flight
    }

    pub(crate) const fn max_in_flight_per_endpoint_value(&self) -> usize {
        self.max_in_flight_per_endpoint
    }

    pub(crate) const fn request_body_limit(&self) -> usize {
        self.max_request_body_bytes
    }

    pub(crate) const fn response_body_limit(&self) -> usize {
        self.max_response_body_bytes
    }

    pub(crate) const fn request_byte_budget(&self) -> usize {
        self.max_inflight_request_body_bytes
    }

    pub(crate) const fn response_byte_budget(&self) -> usize {
        self.max_inflight_response_body_bytes
    }

    pub(crate) fn queue_value(&self) -> &QueueConfig {
        &self.queue
    }

    /// Returns the runtime-wide logical invocation limit.
    pub const fn max_in_flight_value_public(&self) -> usize {
        self.max_in_flight
    }

    /// Returns the per-endpoint attempt limit.
    pub const fn max_in_flight_per_endpoint_value_public(&self) -> usize {
        self.max_in_flight_per_endpoint
    }

    /// Returns the maximum encoded request body size.
    pub const fn max_request_body_bytes_value(&self) -> usize {
        self.max_request_body_bytes
    }

    /// Returns the maximum decoded response body size.
    pub const fn max_response_body_bytes_value(&self) -> usize {
        self.max_response_body_bytes
    }

    /// Returns the global encoded-request byte budget.
    pub const fn max_inflight_request_body_bytes_value(&self) -> usize {
        self.max_inflight_request_body_bytes
    }

    /// Returns the global buffered-response byte budget.
    pub const fn max_inflight_response_body_bytes_value(&self) -> usize {
        self.max_inflight_response_body_bytes
    }

    /// Returns queue behavior.
    pub const fn queue_config(&self) -> &QueueConfig {
        &self.queue
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
    /// Sets the initial ready deadline.
    pub const fn initial_timeout(mut self, value: Duration) -> Self {
        self.initial_timeout = value;
        self
    }

    /// Sets one provider operation timeout.
    pub const fn operation_timeout(mut self, value: Duration) -> Self {
        self.operation_timeout = value;
        self
    }

    /// Sets one subscription close timeout.
    pub const fn close_timeout(mut self, value: Duration) -> Self {
        self.close_timeout = value;
        self
    }

    /// Sets how long the last ready snapshot remains routable after provider disconnection.
    pub const fn max_staleness(mut self, value: Duration) -> Self {
        self.max_staleness = value;
        self
    }

    /// Sets full-jitter reconnect bounds.
    pub const fn reconnect_backoff(mut self, base: Duration, cap: Duration) -> Self {
        self.reconnect_base = base;
        self.reconnect_cap = cap;
        self
    }

    /// Sets the maximum number of shared service subscriptions.
    pub const fn max_subscriptions(mut self, value: usize) -> Self {
        self.max_subscriptions = value;
        self
    }

    pub(crate) const fn initial_timeout_value(&self) -> Duration {
        self.initial_timeout
    }

    pub(crate) const fn operation_timeout_value(&self) -> Duration {
        self.operation_timeout
    }

    pub(crate) const fn close_timeout_value(&self) -> Duration {
        self.close_timeout
    }

    pub(crate) const fn max_staleness_value(&self) -> Duration {
        self.max_staleness
    }

    pub(crate) const fn reconnect_base_value(&self) -> Duration {
        self.reconnect_base
    }

    pub(crate) const fn reconnect_cap_value(&self) -> Duration {
        self.reconnect_cap
    }

    pub(crate) const fn max_subscriptions_value(&self) -> usize {
        self.max_subscriptions
    }

    /// Returns the initial Ready deadline.
    pub const fn initial_timeout_value_public(&self) -> Duration {
        self.initial_timeout
    }
    /// Returns one provider operation timeout.
    pub const fn operation_timeout_value_public(&self) -> Duration {
        self.operation_timeout
    }
    /// Returns one subscription close timeout.
    pub const fn close_timeout_value_public(&self) -> Duration {
        self.close_timeout
    }
    /// Returns the maximum routable stale duration.
    pub const fn max_staleness_value_public(&self) -> Duration {
        self.max_staleness
    }
    /// Returns reconnect backoff bounds.
    pub const fn reconnect_backoff_bounds(&self) -> (Duration, Duration) {
        (self.reconnect_base, self.reconnect_cap)
    }
    /// Returns the shared subscription limit.
    pub const fn max_subscriptions_value_public(&self) -> usize {
        self.max_subscriptions
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
    /// Sets the hard attempt limit, including the first attempt. Valid values are 1 through 3.
    pub const fn max_attempts(mut self, value: u8) -> Self {
        self.max_attempts = value;
        self
    }

    /// Sets full-jitter exponential backoff bounds.
    pub const fn backoff(mut self, base: Duration, cap: Duration) -> Self {
        self.backoff_base = base;
        self.backoff_cap = cap;
        self
    }

    /// Sets the per-service retry token bucket.
    pub const fn budget(mut self, capacity: u32, refill_per_second: u32) -> Self {
        self.budget_capacity = capacity;
        self.budget_refill_per_second = refill_per_second;
        self
    }

    pub(crate) const fn max_attempts_value(&self) -> u8 {
        self.max_attempts
    }

    pub(crate) const fn backoff_base_value(&self) -> Duration {
        self.backoff_base
    }

    pub(crate) const fn backoff_cap_value(&self) -> Duration {
        self.backoff_cap
    }

    pub(crate) const fn budget_capacity_value(&self) -> u32 {
        self.budget_capacity
    }

    pub(crate) const fn budget_refill_value(&self) -> u32 {
        self.budget_refill_per_second
    }

    /// Returns the hard-bounded configured attempt count.
    pub const fn max_attempts_value_public(&self) -> u8 {
        self.max_attempts
    }
    /// Returns full-jitter backoff bounds.
    pub const fn backoff_value(&self) -> (Duration, Duration) {
        (self.backoff_base, self.backoff_cap)
    }
    /// Returns retry token-bucket capacity and refill rate.
    pub const fn budget_value(&self) -> (u32, u32) {
        (self.budget_capacity, self.budget_refill_per_second)
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

    /// Creates a threshold builder from endpoint defaults.
    pub fn endpoint_defaults() -> Self {
        Self::endpoint_default()
    }

    /// Creates a threshold builder from service defaults.
    pub fn service_defaults() -> Self {
        Self::service_default()
    }

    /// Replaces all rolling-window and half-open thresholds.
    #[allow(clippy::too_many_arguments)]
    pub const fn thresholds(
        mut self,
        window: Duration,
        buckets: u8,
        minimum_samples: u32,
        failure_ratio: f64,
        open_duration: Duration,
        half_open_probes: u32,
        close_successes: u32,
    ) -> Self {
        self.window = window;
        self.buckets = buckets;
        self.minimum_samples = minimum_samples;
        self.failure_ratio = failure_ratio;
        self.open_duration = open_duration;
        self.half_open_probes = half_open_probes;
        self.close_successes = close_successes;
        self
    }

    /// Returns the rolling window.
    pub const fn window_value(&self) -> Duration {
        self.window
    }
    /// Returns the bucket count.
    pub const fn buckets_value(&self) -> u8 {
        self.buckets
    }
    /// Returns the minimum sample count.
    pub const fn minimum_samples_value(&self) -> u32 {
        self.minimum_samples
    }
    /// Returns the failure ratio.
    pub const fn failure_ratio_value(&self) -> f64 {
        self.failure_ratio
    }
    /// Returns the initial open duration.
    pub const fn open_duration_value(&self) -> Duration {
        self.open_duration
    }
    /// Returns the half-open concurrency.
    pub const fn half_open_probes_value(&self) -> u32 {
        self.half_open_probes
    }
    /// Returns consecutive successes required to close.
    pub const fn close_successes_value(&self) -> u32 {
        self.close_successes
    }
}

/// Service and endpoint circuit-breaker settings.
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
    /// Replaces endpoint thresholds.
    pub fn endpoint(mut self, value: BreakerThreshold) -> Self {
        self.endpoint = value;
        self
    }

    /// Replaces service thresholds.
    pub fn service(mut self, value: BreakerThreshold) -> Self {
        self.service = value;
        self
    }

    /// Replaces open-duration, endpoint-map, and idle-eviction bounds.
    pub const fn bounds(
        mut self,
        max_open_duration: Duration,
        max_endpoint_entries: usize,
        idle_eviction: Duration,
    ) -> Self {
        self.max_open_duration = max_open_duration;
        self.max_endpoint_entries = max_endpoint_entries;
        self.idle_eviction = idle_eviction;
        self
    }

    /// Returns endpoint thresholds.
    pub const fn endpoint_threshold(&self) -> &BreakerThreshold {
        &self.endpoint
    }
    /// Returns service thresholds.
    pub const fn service_threshold(&self) -> &BreakerThreshold {
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

    pub(crate) fn endpoint_value(&self) -> &BreakerThreshold {
        &self.endpoint
    }

    pub(crate) fn service_value(&self) -> &BreakerThreshold {
        &self.service
    }

    pub(crate) const fn max_open_duration_value(&self) -> Duration {
        self.max_open_duration
    }

    pub(crate) const fn max_endpoint_entries_value(&self) -> usize {
        self.max_endpoint_entries
    }

    pub(crate) const fn idle_eviction_value(&self) -> Duration {
        self.idle_eviction
    }
}

/// Internal HTTP and HTTPS connection-pool behavior.
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
    /// Sets HTTP/1 idle pooling.
    pub const fn http1(mut self, max_idle_per_host: usize, idle_timeout: Option<Duration>) -> Self {
        self.http1_max_idle_per_host = max_idle_per_host;
        self.pool_idle_timeout = idle_timeout;
        self
    }

    /// Sets h2c and TLS/ALPN h2 pool sharding and keep-alive behavior.
    pub const fn http2(
        mut self,
        connections_per_host: usize,
        keep_alive_interval: Option<Duration>,
        keep_alive_timeout: Duration,
    ) -> Self {
        self.http2_connections_per_host = connections_per_host;
        self.http2_keep_alive_interval = keep_alive_interval;
        self.http2_keep_alive_timeout = keep_alive_timeout;
        self
    }

    /// Returns the HTTP/1 idle connection limit.
    pub const fn http1_max_idle_per_host(&self) -> usize {
        self.http1_max_idle_per_host
    }
    /// Returns the shared idle timeout.
    pub const fn pool_idle_timeout(&self) -> Option<Duration> {
        self.pool_idle_timeout
    }
    /// Returns h2c and TLS/ALPN h2 connection shards per endpoint.
    pub const fn http2_connections_per_host(&self) -> usize {
        self.http2_connections_per_host
    }
    /// Returns the h2c and TLS/ALPN h2 keep-alive interval.
    pub const fn http2_keep_alive_interval(&self) -> Option<Duration> {
        self.http2_keep_alive_interval
    }
    /// Returns the h2c and TLS/ALPN h2 keep-alive timeout.
    pub const fn http2_keep_alive_timeout(&self) -> Duration {
        self.http2_keep_alive_timeout
    }
}

impl BreakerThreshold {
    pub(crate) const fn window(&self) -> Duration {
        self.window
    }
    pub(crate) const fn buckets(&self) -> u8 {
        self.buckets
    }
    pub(crate) const fn minimum_samples(&self) -> u32 {
        self.minimum_samples
    }
    pub(crate) const fn failure_ratio(&self) -> f64 {
        self.failure_ratio
    }
    pub(crate) const fn open_duration(&self) -> Duration {
        self.open_duration
    }
    pub(crate) const fn half_open_probes(&self) -> u32 {
        self.half_open_probes
    }
    pub(crate) const fn close_successes(&self) -> u32 {
        self.close_successes
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

    pub(crate) fn validate(&self) -> Result<(), ClientError> {
        let admission = &self.admission;
        let discovery = &self.discovery;
        let retry = &self.retry;
        let breaker = &self.circuit_breaker;
        let positive_durations = [
            self.request_timeout,
            self.connect_timeout,
            self.shutdown_timeout,
            discovery.initial_timeout,
            discovery.operation_timeout,
            discovery.close_timeout,
            discovery.reconnect_base,
            discovery.reconnect_cap,
            retry.backoff_base,
            retry.backoff_cap,
            breaker.max_open_duration,
            breaker.idle_eviction,
            self.http.http2_keep_alive_timeout,
        ];
        if positive_durations.iter().any(Duration::is_zero)
            || admission.max_in_flight == 0
            || admission.max_in_flight_per_endpoint == 0
            || admission.max_request_body_bytes == 0
            || admission.max_response_body_bytes == 0
            || admission.max_inflight_request_body_bytes < admission.max_request_body_bytes
            || admission.max_inflight_response_body_bytes < admission.max_response_body_bytes
            || discovery.max_subscriptions == 0
            || discovery.reconnect_base > discovery.reconnect_cap
            || !(1..=3).contains(&retry.max_attempts)
            || retry.backoff_base > retry.backoff_cap
            || retry.budget_capacity == 0
            || retry.budget_refill_per_second == 0
            || breaker.max_endpoint_entries == 0
            || self.http.http2_connections_per_host == 0
            || self
                .http
                .pool_idle_timeout
                .is_some_and(|duration| duration.is_zero())
            || self
                .http
                .http2_keep_alive_interval
                .is_some_and(|duration| duration.is_zero())
            || [breaker.endpoint_value(), breaker.service_value()]
                .iter()
                .any(|threshold| {
                    threshold.window.is_zero()
                        || threshold.buckets == 0
                        || threshold.minimum_samples == 0
                        || !(0.0..=1.0).contains(&threshold.failure_ratio)
                        || threshold.open_duration.is_zero()
                        || threshold.half_open_probes == 0
                        || threshold.close_successes == 0
                })
            || (admission.queue.capacity > 0 && admission.queue.max_wait.is_zero())
        {
            return Err(ClientError::message(
                ClientErrorKind::Build,
                "client limits, deadlines, and policy values are invalid",
            ));
        }
        Ok(())
    }
}

/// Builder for [`ClientConfig`].
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
    pub fn build(self) -> Result<ClientConfig, ClientError> {
        self.0.validate()?;
        Ok(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_runtime_contract() {
        let config = ClientConfig::default();
        assert_eq!(config.request_timeout(), Duration::from_secs(10));
        assert_eq!(config.retry().max_attempts_value(), 3);
        assert_eq!(config.admission().queue_value().capacity(), 0);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn invalid_attempt_count_is_rejected() {
        let error = ClientConfig::builder()
            .retry(RetryConfig::default().max_attempts(4))
            .build()
            .unwrap_err();
        assert_eq!(error.kind(), ClientErrorKind::Build);
    }
}
