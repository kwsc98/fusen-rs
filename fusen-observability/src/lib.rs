#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Backend-neutral, low-cardinality runtime metrics.
//!
//! Fusen emits structured traces with `tracing` from the runtime itself. This crate defines the
//! metrics boundary used by the runtime and leaves exporter and subscriber ownership to the
//! application. Implementations must be non-blocking; the runtime disables a recorder after its
//! first panic.

use std::sync::Arc;
use std::time::Duration;

/// OpenTelemetry metrics adapter. Applications retain ownership of their provider/exporter guard.
#[cfg(feature = "otel")]
pub mod otel;

/// The side of an observed service invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetricSide {
    /// An outbound client invocation.
    Client,
    /// An inbound server invocation.
    Server,
}

/// A terminal low-cardinality outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetricOutcome {
    /// Work completed successfully.
    Success,
    /// Work completed with an error.
    Error,
    /// A deadline elapsed.
    Timeout,
    /// The caller cancelled the work.
    Cancelled,
    /// Admission or a resource budget rejected the work.
    Rejected,
}

/// The state of a circuit breaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CircuitState {
    /// Requests are admitted normally.
    Closed,
    /// Requests fail fast until the open interval expires.
    Open,
    /// A bounded number of recovery probes are admitted.
    HalfOpen,
}

/// The state of a discovery directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectoryMetricState {
    /// Initial discovery has not completed.
    Initializing,
    /// The directory has a current provider snapshot.
    Ready,
    /// The provider stream is unavailable but the last snapshot remains within its grace period.
    Stale,
    /// No snapshot is eligible for routing.
    Unavailable,
    /// The directory has terminated.
    Closed,
}

/// A logical invocation entering admission.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct InvocationStartedEvent<'a> {
    side: MetricSide,
    binding: &'a str,
    http_version: Option<&'a str>,
    service: &'a str,
    method: &'a str,
}

impl<'a> InvocationStartedEvent<'a> {
    /// Creates an invocation-started event.
    pub const fn new(
        side: MetricSide,
        binding: &'a str,
        http_version: Option<&'a str>,
        service: &'a str,
        method: &'a str,
    ) -> Self {
        Self {
            side,
            binding,
            http_version,
            service,
            method,
        }
    }
    /// Returns the client/server side.
    pub const fn side(&self) -> MetricSide {
        self.side
    }
    /// Returns the stable HTTP binding identifier.
    pub const fn binding(&self) -> &'a str {
        self.binding
    }
    /// Returns the actual HTTP version when this event represents physical server work.
    pub const fn http_version(&self) -> Option<&'a str> {
        self.http_version
    }
    /// Returns the interface identifier.
    pub const fn service(&self) -> &'a str {
        self.service
    }
    /// Returns the method name.
    pub const fn method(&self) -> &'a str {
        self.method
    }
}

/// A logical invocation's terminal outcome.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct InvocationFinishedEvent<'a> {
    side: MetricSide,
    binding: &'a str,
    http_version: Option<&'a str>,
    service: &'a str,
    method: &'a str,
    outcome: MetricOutcome,
    status_class: Option<&'a str>,
    error_code: Option<&'a str>,
    duration: Duration,
    attempts: u8,
}

impl<'a> InvocationFinishedEvent<'a> {
    /// Creates an invocation-finished event.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        side: MetricSide,
        binding: &'a str,
        http_version: Option<&'a str>,
        service: &'a str,
        method: &'a str,
        outcome: MetricOutcome,
        status_class: Option<&'a str>,
        error_code: Option<&'a str>,
        duration: Duration,
        attempts: u8,
    ) -> Self {
        Self {
            side,
            binding,
            http_version,
            service,
            method,
            outcome,
            status_class,
            error_code,
            duration,
            attempts,
        }
    }
    /// Returns the client/server side.
    pub const fn side(&self) -> MetricSide {
        self.side
    }
    /// Returns the stable HTTP binding identifier.
    pub const fn binding(&self) -> &'a str {
        self.binding
    }
    /// Returns the actual HTTP version when this event represents physical server work.
    pub const fn http_version(&self) -> Option<&'a str> {
        self.http_version
    }
    /// Returns the interface identifier.
    pub const fn service(&self) -> &'a str {
        self.service
    }
    /// Returns the method name.
    pub const fn method(&self) -> &'a str {
        self.method
    }
    /// Returns the terminal outcome.
    pub const fn outcome(&self) -> MetricOutcome {
        self.outcome
    }
    /// Returns the HTTP status class.
    pub const fn status_class(&self) -> Option<&'a str> {
        self.status_class
    }
    /// Returns the stable error code.
    pub const fn error_code(&self) -> Option<&'a str> {
        self.error_code
    }
    /// Returns the end-to-end duration.
    pub const fn duration(&self) -> Duration {
        self.duration
    }
    /// Returns the number of attempts.
    pub const fn attempts(&self) -> u8 {
        self.attempts
    }
}

/// A physical transport attempt's terminal outcome.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct AttemptFinishedEvent<'a> {
    binding: &'a str,
    http_version: Option<&'a str>,
    service: &'a str,
    method: &'a str,
    attempt: u8,
    outcome: MetricOutcome,
    failure_class: Option<&'a str>,
    duration: Duration,
}

impl<'a> AttemptFinishedEvent<'a> {
    /// Creates an attempt-finished event.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        binding: &'a str,
        http_version: Option<&'a str>,
        service: &'a str,
        method: &'a str,
        attempt: u8,
        outcome: MetricOutcome,
        failure_class: Option<&'a str>,
        duration: Duration,
    ) -> Self {
        Self {
            binding,
            http_version,
            service,
            method,
            attempt,
            outcome,
            failure_class,
            duration,
        }
    }
    /// Returns the stable HTTP binding identifier.
    pub const fn binding(&self) -> &'a str {
        self.binding
    }
    /// Returns the actual HTTP version, or `None` when the attempt failed before one was known.
    pub const fn http_version(&self) -> Option<&'a str> {
        self.http_version
    }
    /// Returns the interface identifier.
    pub const fn service(&self) -> &'a str {
        self.service
    }
    /// Returns the method name.
    pub const fn method(&self) -> &'a str {
        self.method
    }
    /// Returns the attempt number.
    pub const fn attempt(&self) -> u8 {
        self.attempt
    }
    /// Returns the terminal outcome.
    pub const fn outcome(&self) -> MetricOutcome {
        self.outcome
    }
    /// Returns the failure class.
    pub const fn failure_class(&self) -> Option<&'a str> {
        self.failure_class
    }
    /// Returns the attempt duration.
    pub const fn duration(&self) -> Duration {
        self.duration
    }
}

/// An admission or bounded-resource rejection.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct AdmissionRejectedEvent<'a> {
    side: MetricSide,
    reason: &'a str,
}

impl<'a> AdmissionRejectedEvent<'a> {
    /// Creates an admission-rejected event.
    pub const fn new(side: MetricSide, reason: &'a str) -> Self {
        Self { side, reason }
    }
    /// Returns the client/server side.
    pub const fn side(&self) -> MetricSide {
        self.side
    }
    /// Returns the static rejection reason.
    pub const fn reason(&self) -> &'a str {
        self.reason
    }
}

/// A registry lifecycle operation's terminal outcome.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RegistryOperationEvent<'a> {
    registry: &'a str,
    operation: &'a str,
    outcome: MetricOutcome,
    duration: Duration,
}

impl<'a> RegistryOperationEvent<'a> {
    /// Creates a registry-operation event.
    pub const fn new(
        registry: &'a str,
        operation: &'a str,
        outcome: MetricOutcome,
        duration: Duration,
    ) -> Self {
        Self {
            registry,
            operation,
            outcome,
            duration,
        }
    }
    /// Returns the registry name.
    pub const fn registry(&self) -> &'a str {
        self.registry
    }
    /// Returns the operation name.
    pub const fn operation(&self) -> &'a str {
        self.operation
    }
    /// Returns the terminal outcome.
    pub const fn outcome(&self) -> MetricOutcome {
        self.outcome
    }
    /// Returns the operation duration.
    pub const fn duration(&self) -> Duration {
        self.duration
    }
}

/// A discovery directory state transition.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct DirectoryStateChangedEvent<'a> {
    service: &'a str,
    state: DirectoryMetricState,
}

impl<'a> DirectoryStateChangedEvent<'a> {
    /// Creates a directory-state event.
    pub const fn new(service: &'a str, state: DirectoryMetricState) -> Self {
        Self { service, state }
    }
    /// Returns the interface identifier.
    pub const fn service(&self) -> &'a str {
        self.service
    }
    /// Returns the new directory state.
    pub const fn state(&self) -> DirectoryMetricState {
        self.state
    }
}

/// A circuit-breaker state transition.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct CircuitStateChangedEvent<'a> {
    scope: &'a str,
    binding: &'a str,
    service: &'a str,
    state: CircuitState,
}

impl<'a> CircuitStateChangedEvent<'a> {
    /// Creates a circuit-state event.
    pub const fn new(
        scope: &'a str,
        binding: &'a str,
        service: &'a str,
        state: CircuitState,
    ) -> Self {
        Self {
            scope,
            binding,
            service,
            state,
        }
    }
    /// Returns `service` or `endpoint`.
    pub const fn scope(&self) -> &'a str {
        self.scope
    }
    /// Returns the HTTP binding identifier.
    pub const fn binding(&self) -> &'a str {
        self.binding
    }
    /// Returns the interface identifier.
    pub const fn service(&self) -> &'a str {
        self.service
    }
    /// Returns the new circuit state.
    pub const fn state(&self) -> CircuitState {
        self.state
    }
}

/// A client or server shutdown's terminal outcome.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ShutdownFinishedEvent<'a> {
    runtime: &'a str,
    outcome: MetricOutcome,
    duration: Duration,
}

impl<'a> ShutdownFinishedEvent<'a> {
    /// Creates a shutdown-finished event.
    pub const fn new(runtime: &'a str, outcome: MetricOutcome, duration: Duration) -> Self {
        Self {
            runtime,
            outcome,
            duration,
        }
    }
    /// Returns `client` or `server`.
    pub const fn runtime(&self) -> &'a str {
        self.runtime
    }
    /// Returns the terminal outcome.
    pub const fn outcome(&self) -> MetricOutcome {
        self.outcome
    }
    /// Returns the shutdown duration.
    pub const fn duration(&self) -> Duration {
        self.duration
    }
}

/// One low-cardinality runtime measurement.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum MetricEvent<'a> {
    /// A logical invocation entered admission.
    InvocationStarted(InvocationStartedEvent<'a>),
    /// A logical invocation reached its terminal outcome.
    InvocationFinished(InvocationFinishedEvent<'a>),
    /// One physical transport attempt completed.
    AttemptFinished(AttemptFinishedEvent<'a>),
    /// Admission or a bounded resource rejected work.
    AdmissionRejected(AdmissionRejectedEvent<'a>),
    /// One registry lifecycle operation completed.
    RegistryOperation(RegistryOperationEvent<'a>),
    /// A discovery directory changed state.
    DirectoryStateChanged(DirectoryStateChangedEvent<'a>),
    /// A service or endpoint circuit changed state.
    CircuitStateChanged(CircuitStateChangedEvent<'a>),
    /// A client or server shutdown completed.
    ShutdownFinished(ShutdownFinishedEvent<'a>),
}

/// Synchronous sink for low-cardinality runtime metrics.
///
/// Implementations must not block. Events never contain request IDs, endpoint addresses, bodies,
/// credentials, full headers, provider error text, or other unbounded user-controlled labels.
pub trait MetricsRecorder: Send + Sync + 'static {
    /// Records one measurement.
    fn record(&self, event: &MetricEvent<'_>);
}

impl<T> MetricsRecorder for Arc<T>
where
    T: MetricsRecorder + ?Sized,
{
    fn record(&self, event: &MetricEvent<'_>) {
        (**self).record(event);
    }
}

/// A recorder that intentionally discards every event.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopMetricsRecorder;

impl MetricsRecorder for NoopMetricsRecorder {
    fn record(&self, _event: &MetricEvent<'_>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_object_safe(_: &dyn MetricsRecorder) {}

    #[test]
    fn recorder_is_object_safe() {
        assert_object_safe(&NoopMetricsRecorder);
    }

    #[test]
    fn debug_output_does_not_require_high_cardinality_fields() {
        let event = MetricEvent::AdmissionRejected(AdmissionRejectedEvent::new(
            MetricSide::Server,
            "concurrency",
        ));
        assert_eq!(
            format!("{event:?}"),
            "AdmissionRejected(AdmissionRejectedEvent { side: Server, reason: \"concurrency\" })"
        );
    }

    #[test]
    fn attempt_http_version_can_be_unknown_before_protocol_establishment() {
        let unknown = AttemptFinishedEvent::new(
            "http-json-v1",
            None,
            "service",
            "call",
            1,
            MetricOutcome::Error,
            Some("connect"),
            Duration::ZERO,
        );
        assert_eq!(unknown.http_version(), None);

        let known = AttemptFinishedEvent::new(
            "http-json-v1",
            Some("2"),
            "service",
            "call",
            1,
            MetricOutcome::Success,
            None,
            Duration::ZERO,
        );
        assert_eq!(known.http_version(), Some("2"));
    }

    #[test]
    fn circuit_state_changes_include_the_http_binding() {
        let event =
            CircuitStateChangedEvent::new("service", "http-json-v1", "service", CircuitState::Open);

        assert_eq!(event.scope(), "service");
        assert_eq!(event.binding(), "http-json-v1");
        assert_eq!(event.service(), "service");
        assert_eq!(event.state(), CircuitState::Open);
    }
}
