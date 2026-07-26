#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Backend-neutral, low-cardinality runtime metrics.
//!
//! Fusen emits structured traces with `tracing` from the runtime itself. This crate defines the
//! metrics boundary used by the runtime and leaves exporter and subscriber ownership to the
//! application. Implementations must be non-blocking; the runtime disables a recorder after its
//! first panic.

use std::time::Duration;

/// OpenTelemetry metrics adapter. Applications retain ownership of their provider/exporter guard.
#[cfg(feature = "otel")]
pub mod otel;

/// The side of an observed RPC.
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

/// One low-cardinality runtime measurement.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum MetricEvent<'a> {
    /// A logical invocation entered admission.
    InvocationStarted {
        /// Client or server side.
        side: MetricSide,
        /// Stable wire protocol name.
        protocol: &'a str,
        /// Static service identifier.
        service: &'a str,
        /// Static method name.
        method: &'a str,
    },
    /// A logical invocation reached its one terminal outcome.
    InvocationFinished {
        /// Client or server side.
        side: MetricSide,
        /// Stable wire protocol name.
        protocol: &'a str,
        /// Static service identifier.
        service: &'a str,
        /// Static method name.
        method: &'a str,
        /// Terminal outcome.
        outcome: MetricOutcome,
        /// HTTP status class (`2xx`, `4xx`, and so on), when available.
        status_class: Option<&'a str>,
        /// Stable framework or application error code, when available.
        error_code: Option<&'a str>,
        /// End-to-end logical duration.
        duration: Duration,
        /// Number of physical attempts made by the invocation.
        attempts: u8,
    },
    /// One physical transport attempt completed.
    AttemptFinished {
        /// Stable protocol name.
        protocol: &'a str,
        /// Static service identifier.
        service: &'a str,
        /// Static method name.
        method: &'a str,
        /// Attempt number, starting at one.
        attempt: u8,
        /// Terminal attempt outcome.
        outcome: MetricOutcome,
        /// Typed, low-cardinality failure class.
        failure_class: Option<&'a str>,
        /// Attempt duration.
        duration: Duration,
    },
    /// Admission or a bounded resource rejected work.
    AdmissionRejected {
        /// Client or server side.
        side: MetricSide,
        /// Static rejection class such as `concurrency` or `body_bytes`.
        reason: &'a str,
    },
    /// One registry lifecycle operation completed.
    RegistryOperation {
        /// Application-supplied, validated registry name.
        registry: &'a str,
        /// Static operation name.
        operation: &'a str,
        /// Terminal outcome.
        outcome: MetricOutcome,
        /// Operation duration.
        duration: Duration,
    },
    /// A discovery directory changed state.
    DirectoryStateChanged {
        /// Static service identifier.
        service: &'a str,
        /// New state.
        state: DirectoryMetricState,
    },
    /// A service or endpoint circuit changed state.
    CircuitStateChanged {
        /// `service` or `endpoint`; endpoint identity is intentionally omitted.
        scope: &'a str,
        /// Static service identifier.
        service: &'a str,
        /// New state.
        state: CircuitState,
    },
    /// A client or server shutdown completed.
    ShutdownFinished {
        /// `client` or `server`.
        runtime: &'a str,
        /// Terminal outcome.
        outcome: MetricOutcome,
        /// Shutdown duration.
        duration: Duration,
    },
}

/// Synchronous sink for low-cardinality runtime metrics.
///
/// Implementations must not block. Events never contain request IDs, endpoint addresses, bodies,
/// credentials, full headers, provider error text, or other unbounded user-controlled labels.
pub trait MetricsRecorder: Send + Sync + 'static {
    /// Records one measurement.
    fn record(&self, event: &MetricEvent<'_>);
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
        let event = MetricEvent::AdmissionRejected {
            side: MetricSide::Server,
            reason: "concurrency",
        };
        assert_eq!(
            format!("{event:?}"),
            "AdmissionRejected { side: Server, reason: \"concurrency\" }"
        );
    }
}
