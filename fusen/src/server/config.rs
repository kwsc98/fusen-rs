use crate::error::{ServerError, ServerErrorKind};
use fusen_contract::ProtocolSet;
use std::time::Duration;

const MIB: usize = 1024 * 1024;

/// Server request admission, deadline, and body limits.
#[derive(Clone, Debug)]
pub struct ServerRequestConfig {
    timeout: Duration,
    max_concurrent_requests: usize,
    max_request_body_bytes: usize,
    max_response_body_bytes: usize,
    max_inflight_request_body_bytes: usize,
    max_inflight_response_body_bytes: usize,
    queue_capacity: usize,
    queue_max_wait: Duration,
}

impl Default for ServerRequestConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_concurrent_requests: 1024,
            max_request_body_bytes: 2 * MIB,
            max_response_body_bytes: 2 * MIB,
            max_inflight_request_body_bytes: 64 * MIB,
            max_inflight_response_body_bytes: 64 * MIB,
            queue_capacity: 0,
            queue_max_wait: Duration::from_millis(50),
        }
    }
}

impl ServerRequestConfig {
    /// Sets the local upper bound for one request deadline.
    pub const fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }

    /// Sets the maximum number of admitted requests.
    pub const fn max_concurrent_requests(mut self, value: usize) -> Self {
        self.max_concurrent_requests = value;
        self
    }

    /// Sets per-request and per-response JSON limits.
    pub const fn body_limits(mut self, request: usize, response: usize) -> Self {
        self.max_request_body_bytes = request;
        self.max_response_body_bytes = response;
        self
    }

    /// Sets runtime-wide request and response byte budgets.
    pub const fn inflight_body_budgets(mut self, request: usize, response: usize) -> Self {
        self.max_inflight_request_body_bytes = request;
        self.max_inflight_response_body_bytes = response;
        self
    }

    /// Enables a bounded admission queue. Zero preserves fail-fast behavior.
    pub const fn queue(mut self, capacity: usize, max_wait: Duration) -> Self {
        self.queue_capacity = capacity;
        self.queue_max_wait = max_wait;
        self
    }

    /// Returns the local request deadline cap.
    pub const fn request_timeout(&self) -> Duration {
        self.timeout
    }

    /// Returns the server-wide in-flight request limit.
    pub const fn max_concurrent_requests_value(&self) -> usize {
        self.max_concurrent_requests
    }

    /// Returns the maximum request body size.
    pub const fn max_request_body_bytes(&self) -> usize {
        self.max_request_body_bytes
    }

    /// Returns the maximum response body size.
    pub const fn max_response_body_bytes(&self) -> usize {
        self.max_response_body_bytes
    }

    /// Returns the global buffered-request byte budget.
    pub const fn max_inflight_request_body_bytes(&self) -> usize {
        self.max_inflight_request_body_bytes
    }

    /// Returns the global buffered-response byte budget.
    pub const fn max_inflight_response_body_bytes(&self) -> usize {
        self.max_inflight_response_body_bytes
    }

    /// Returns the optional queue capacity.
    pub const fn queue_capacity(&self) -> usize {
        self.queue_capacity
    }

    /// Returns the queue wait cap.
    pub const fn queue_max_wait(&self) -> Duration {
        self.queue_max_wait
    }
}

/// Plain HTTP/1.1 and h2c server settings.
#[derive(Clone, Debug)]
pub struct HttpServerConfig {
    max_connections: usize,
    max_uri_bytes: usize,
    max_query_pairs: usize,
    max_headers: usize,
    max_header_bytes: usize,
    http1_header_read_timeout: Duration,
    http2_max_concurrent_streams: u32,
    http2_keep_alive_interval: Option<Duration>,
    http2_keep_alive_timeout: Duration,
}

impl Default for HttpServerConfig {
    fn default() -> Self {
        Self {
            max_connections: 2048,
            max_uri_bytes: 8 * 1024,
            max_query_pairs: 128,
            max_headers: 64,
            max_header_bytes: 32 * 1024,
            http1_header_read_timeout: Duration::from_secs(10),
            http2_max_concurrent_streams: 128,
            http2_keep_alive_interval: Some(Duration::from_secs(30)),
            http2_keep_alive_timeout: Duration::from_secs(10),
        }
    }
}

impl HttpServerConfig {
    /// Sets the accepted TCP connection limit.
    pub const fn max_connections(mut self, value: usize) -> Self {
        self.max_connections = value;
        self
    }

    /// Sets URI, query-pair, header-count, and aggregate header limits.
    pub const fn head_limits(
        mut self,
        uri_bytes: usize,
        query_pairs: usize,
        headers: usize,
        header_bytes: usize,
    ) -> Self {
        self.max_uri_bytes = uri_bytes;
        self.max_query_pairs = query_pairs;
        self.max_headers = headers;
        self.max_header_bytes = header_bytes;
        self
    }

    /// Sets the HTTP/1.1 header deadline.
    pub const fn http1_header_read_timeout(mut self, value: Duration) -> Self {
        self.http1_header_read_timeout = value;
        self
    }

    /// Sets HTTP/2 stream and keep-alive behavior.
    pub const fn http2(
        mut self,
        max_streams: u32,
        keep_alive_interval: Option<Duration>,
        keep_alive_timeout: Duration,
    ) -> Self {
        self.http2_max_concurrent_streams = max_streams;
        self.http2_keep_alive_interval = keep_alive_interval;
        self.http2_keep_alive_timeout = keep_alive_timeout;
        self
    }

    /// Returns the TCP connection limit.
    pub const fn max_connections_value(&self) -> usize {
        self.max_connections
    }

    /// Returns the request URI byte limit.
    pub const fn max_uri_bytes(&self) -> usize {
        self.max_uri_bytes
    }

    /// Returns the query pair limit.
    pub const fn max_query_pairs(&self) -> usize {
        self.max_query_pairs
    }

    /// Returns the header count limit.
    pub const fn max_headers(&self) -> usize {
        self.max_headers
    }

    /// Returns the aggregate header byte limit.
    pub const fn max_header_bytes(&self) -> usize {
        self.max_header_bytes
    }

    /// Returns the HTTP/1.1 header deadline.
    pub const fn http1_header_read_timeout_value(&self) -> Duration {
        self.http1_header_read_timeout
    }

    /// Returns the HTTP/2 stream limit.
    pub const fn http2_max_concurrent_streams(&self) -> u32 {
        self.http2_max_concurrent_streams
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

/// Registry startup operation limits.
#[derive(Clone, Debug)]
pub struct ServerRegistryConfig {
    startup_timeout: Duration,
    operation_timeout: Duration,
    max_concurrent_operations: usize,
}

impl Default for ServerRegistryConfig {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(30),
            operation_timeout: Duration::from_secs(5),
            max_concurrent_operations: 8,
        }
    }
}

impl ServerRegistryConfig {
    /// Sets startup, per-operation, and concurrency limits.
    pub const fn limits(
        mut self,
        startup_timeout: Duration,
        operation_timeout: Duration,
        max_concurrent_operations: usize,
    ) -> Self {
        self.startup_timeout = startup_timeout;
        self.operation_timeout = operation_timeout;
        self.max_concurrent_operations = max_concurrent_operations;
        self
    }

    /// Returns the total Ready deadline.
    pub const fn startup_timeout(&self) -> Duration {
        self.startup_timeout
    }

    /// Returns one registry operation deadline.
    pub const fn operation_timeout(&self) -> Duration {
        self.operation_timeout
    }

    /// Returns the activation/close concurrency window.
    pub const fn max_concurrent_operations(&self) -> usize {
        self.max_concurrent_operations
    }
}

/// Immutable production server configuration.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    protocols: ProtocolSet,
    request: ServerRequestConfig,
    http: HttpServerConfig,
    registry: ServerRegistryConfig,
    graceful_shutdown_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            protocols: ProtocolSet::FUSEN_V1,
            request: ServerRequestConfig::default(),
            http: HttpServerConfig::default(),
            registry: ServerRegistryConfig::default(),
            graceful_shutdown_timeout: Duration::from_secs(30),
        }
    }
}

impl ServerConfig {
    /// Starts a builder with bounded production defaults.
    pub fn builder() -> ServerConfigBuilder {
        ServerConfigBuilder(Self::default())
    }

    /// Returns enabled protocols.
    pub const fn protocols(&self) -> ProtocolSet {
        self.protocols
    }

    /// Returns request limits.
    pub const fn request(&self) -> &ServerRequestConfig {
        &self.request
    }

    /// Returns HTTP limits.
    pub const fn http(&self) -> &HttpServerConfig {
        &self.http
    }

    /// Returns registry lifecycle limits.
    pub const fn registry(&self) -> &ServerRegistryConfig {
        &self.registry
    }

    /// Returns the deadline shared by deregistration and connection drain.
    pub const fn graceful_shutdown_timeout(&self) -> Duration {
        self.graceful_shutdown_timeout
    }

    pub(crate) fn validate(&self) -> Result<(), ServerError> {
        let request = &self.request;
        let http = &self.http;
        let registry = &self.registry;
        if request.timeout.is_zero()
            || request.max_concurrent_requests == 0
            || request.max_request_body_bytes == 0
            || request.max_response_body_bytes == 0
            || request.max_inflight_request_body_bytes < request.max_request_body_bytes
            || request.max_inflight_response_body_bytes < request.max_response_body_bytes
            || (request.queue_capacity > 0 && request.queue_max_wait.is_zero())
            || http.max_connections == 0
            || http.max_uri_bytes == 0
            || http.max_query_pairs == 0
            || http.max_headers == 0
            || http.max_header_bytes == 0
            || http.http1_header_read_timeout.is_zero()
            || http.http2_max_concurrent_streams == 0
            || http.http2_keep_alive_timeout.is_zero()
            || registry.startup_timeout.is_zero()
            || registry.operation_timeout.is_zero()
            || registry.max_concurrent_operations == 0
            || self.graceful_shutdown_timeout.is_zero()
        {
            return Err(ServerError::message(
                ServerErrorKind::Validation,
                "server limits and deadlines must be positive and internally consistent",
            ));
        }
        Ok(())
    }
}

/// Builder for [`ServerConfig`].
pub struct ServerConfigBuilder(ServerConfig);

impl ServerConfigBuilder {
    /// Replaces enabled protocols.
    pub const fn protocols(mut self, value: ProtocolSet) -> Self {
        self.0.protocols = value;
        self
    }

    /// Replaces request limits.
    pub fn request(mut self, value: ServerRequestConfig) -> Self {
        self.0.request = value;
        self
    }

    /// Replaces HTTP limits.
    pub fn http(mut self, value: HttpServerConfig) -> Self {
        self.0.http = value;
        self
    }

    /// Replaces registry limits.
    pub fn registry(mut self, value: ServerRegistryConfig) -> Self {
        self.0.registry = value;
        self
    }

    /// Sets the total graceful shutdown budget.
    pub const fn graceful_shutdown_timeout(mut self, value: Duration) -> Self {
        self.0.graceful_shutdown_timeout = value;
        self
    }

    /// Validates and builds the configuration.
    pub fn build(self) -> Result<ServerConfig, ServerError> {
        self.0.validate()?;
        Ok(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Debug;

    fn assert_default_cases<T>(cases: &[(&str, T, T)])
    where
        T: Debug + PartialEq,
    {
        for (name, actual, expected) in cases {
            assert_eq!(actual, expected, "unexpected default for {name}");
        }
    }

    #[test]
    fn public_default_getters_match_the_runtime_contract() {
        let config = ServerConfig::default();
        let request = config.request();
        let http = config.http();
        let registry = config.registry();

        assert_default_cases(&[(
            "server.protocols",
            config.protocols(),
            ProtocolSet::FUSEN_V1,
        )]);
        assert_default_cases(&[
            (
                "request.timeout",
                request.request_timeout(),
                Duration::from_secs(30),
            ),
            (
                "request.queue_max_wait",
                request.queue_max_wait(),
                Duration::from_millis(50),
            ),
            (
                "http.http1_header_read_timeout",
                http.http1_header_read_timeout_value(),
                Duration::from_secs(10),
            ),
            (
                "http.http2_keep_alive_timeout",
                http.http2_keep_alive_timeout(),
                Duration::from_secs(10),
            ),
            (
                "registry.startup_timeout",
                registry.startup_timeout(),
                Duration::from_secs(30),
            ),
            (
                "registry.operation_timeout",
                registry.operation_timeout(),
                Duration::from_secs(5),
            ),
            (
                "server.graceful_shutdown_timeout",
                config.graceful_shutdown_timeout(),
                Duration::from_secs(30),
            ),
        ]);
        assert_default_cases(&[
            (
                "request.max_concurrent_requests",
                request.max_concurrent_requests_value(),
                1024,
            ),
            (
                "request.max_request_body_bytes",
                request.max_request_body_bytes(),
                2 * MIB,
            ),
            (
                "request.max_response_body_bytes",
                request.max_response_body_bytes(),
                2 * MIB,
            ),
            (
                "request.max_inflight_request_body_bytes",
                request.max_inflight_request_body_bytes(),
                64 * MIB,
            ),
            (
                "request.max_inflight_response_body_bytes",
                request.max_inflight_response_body_bytes(),
                64 * MIB,
            ),
            ("request.queue_capacity", request.queue_capacity(), 0),
            ("http.max_connections", http.max_connections_value(), 2048),
            ("http.max_uri_bytes", http.max_uri_bytes(), 8 * 1024),
            ("http.max_query_pairs", http.max_query_pairs(), 128),
            ("http.max_headers", http.max_headers(), 64),
            ("http.max_header_bytes", http.max_header_bytes(), 32 * 1024),
            (
                "registry.max_concurrent_operations",
                registry.max_concurrent_operations(),
                8,
            ),
        ]);
        assert_default_cases(&[(
            "http.http2_max_concurrent_streams",
            http.http2_max_concurrent_streams(),
            128_u32,
        )]);
        assert_default_cases(&[(
            "http.http2_keep_alive_interval",
            http.http2_keep_alive_interval(),
            Some(Duration::from_secs(30)),
        )]);
    }

    #[test]
    fn invalid_limits_return_a_typed_validation_error() {
        let error = ServerConfig::builder()
            .request(ServerRequestConfig::default().max_concurrent_requests(0))
            .build()
            .unwrap_err();
        assert_eq!(error.kind(), ServerErrorKind::Validation);
    }

    #[test]
    fn cross_field_boundaries_pass_and_one_step_overages_fail() {
        const BODY_BUDGET: usize = 1024;

        let cases = [
            (
                "request body limit and global budget",
                ServerConfig::builder()
                    .request(
                        ServerRequestConfig::default()
                            .body_limits(BODY_BUDGET, 2 * MIB)
                            .inflight_body_budgets(BODY_BUDGET, 64 * MIB),
                    )
                    .build(),
                ServerConfig::builder()
                    .request(
                        ServerRequestConfig::default()
                            .body_limits(BODY_BUDGET + 1, 2 * MIB)
                            .inflight_body_budgets(BODY_BUDGET, 64 * MIB),
                    )
                    .build(),
            ),
            (
                "response body limit and global budget",
                ServerConfig::builder()
                    .request(
                        ServerRequestConfig::default()
                            .body_limits(2 * MIB, BODY_BUDGET)
                            .inflight_body_budgets(64 * MIB, BODY_BUDGET),
                    )
                    .build(),
                ServerConfig::builder()
                    .request(
                        ServerRequestConfig::default()
                            .body_limits(2 * MIB, BODY_BUDGET + 1)
                            .inflight_body_budgets(64 * MIB, BODY_BUDGET),
                    )
                    .build(),
            ),
            (
                "queue capacity and wait",
                ServerConfig::builder()
                    .request(ServerRequestConfig::default().queue(0, Duration::ZERO))
                    .build(),
                ServerConfig::builder()
                    .request(ServerRequestConfig::default().queue(1, Duration::ZERO))
                    .build(),
            ),
        ];

        for (name, boundary, one_step_over) in cases {
            assert!(
                boundary.is_ok(),
                "{name} equality/disabled boundary must be accepted: {boundary:?}"
            );
            let error = one_step_over.expect_err("one-step overage must be rejected");
            assert_eq!(error.kind(), ServerErrorKind::Validation, "{name}");
        }
    }
}
