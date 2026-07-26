//! OpenTelemetry instruments for Fusen's bounded metric event vocabulary.

use crate::{
    CircuitState, DirectoryMetricState, MetricEvent, MetricOutcome, MetricSide, MetricsRecorder,
};
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram, Meter},
};

/// Records Fusen metric events into an application-owned OpenTelemetry [`Meter`].
///
/// This adapter never installs a global provider or exporter. The application must construct and
/// retain its telemetry provider/exporter guard for at least as long as this recorder is in use.
#[derive(Clone, Debug)]
pub struct OpenTelemetryMetricsRecorder {
    events: Counter<u64>,
    duration_seconds: Histogram<f64>,
    attempts: Histogram<u64>,
}

impl OpenTelemetryMetricsRecorder {
    /// Builds Fusen instruments in the supplied application-owned meter.
    pub fn new(meter: &Meter) -> Self {
        Self {
            events: meter
                .u64_counter("fusen.runtime.events")
                .with_description("Fusen runtime events by stable low-cardinality outcome")
                .build(),
            duration_seconds: meter
                .f64_histogram("fusen.runtime.duration")
                .with_description("Fusen operation duration in seconds")
                .build(),
            attempts: meter
                .u64_histogram("fusen.rpc.attempts")
                .with_description("Physical attempts per logical RPC invocation")
                .build(),
        }
    }

    fn record_event(&self, attributes: &[KeyValue]) {
        self.events.add(1, attributes);
    }

    fn record_duration(&self, duration: std::time::Duration, attributes: &[KeyValue]) {
        self.duration_seconds
            .record(duration.as_secs_f64(), attributes);
    }
}

impl MetricsRecorder for OpenTelemetryMetricsRecorder {
    fn record(&self, event: &MetricEvent<'_>) {
        match event {
            MetricEvent::InvocationStarted {
                side,
                protocol,
                service,
                method,
            } => {
                let attributes =
                    invocation_attributes("invocation_started", *side, protocol, service, method);
                self.record_event(&attributes);
            }
            MetricEvent::InvocationFinished {
                side,
                protocol,
                service,
                method,
                outcome,
                status_class,
                error_code,
                duration,
                attempts,
            } => {
                let mut attributes =
                    invocation_attributes("invocation_finished", *side, protocol, service, method);
                attributes.push(KeyValue::new("outcome", outcome_name(*outcome)));
                if let Some(status_class) = status_class {
                    attributes.push(KeyValue::new("status_class", (*status_class).to_owned()));
                }
                if let Some(error_code) = error_code {
                    attributes.push(KeyValue::new("error_code", (*error_code).to_owned()));
                }
                self.record_event(&attributes);
                self.record_duration(*duration, &attributes);
                self.attempts.record(u64::from(*attempts), &attributes);
            }
            MetricEvent::AttemptFinished {
                protocol,
                service,
                method,
                attempt,
                outcome,
                failure_class,
                duration,
            } => {
                let mut attributes = vec![
                    KeyValue::new("event", "attempt_finished"),
                    KeyValue::new("protocol", (*protocol).to_owned()),
                    KeyValue::new("service", (*service).to_owned()),
                    KeyValue::new("method", (*method).to_owned()),
                    KeyValue::new("outcome", outcome_name(*outcome)),
                    KeyValue::new("attempt", i64::from(*attempt)),
                ];
                if let Some(failure_class) = failure_class {
                    attributes.push(KeyValue::new("failure_class", (*failure_class).to_owned()));
                }
                self.record_event(&attributes);
                self.record_duration(*duration, &attributes);
            }
            MetricEvent::AdmissionRejected { side, reason } => self.record_event(&[
                KeyValue::new("event", "admission_rejected"),
                KeyValue::new("side", side_name(*side)),
                KeyValue::new("reason", (*reason).to_owned()),
            ]),
            MetricEvent::RegistryOperation {
                registry,
                operation,
                outcome,
                duration,
            } => {
                let attributes = [
                    KeyValue::new("event", "registry_operation"),
                    KeyValue::new("registry", (*registry).to_owned()),
                    KeyValue::new("operation", (*operation).to_owned()),
                    KeyValue::new("outcome", outcome_name(*outcome)),
                ];
                self.record_event(&attributes);
                self.record_duration(*duration, &attributes);
            }
            MetricEvent::DirectoryStateChanged { service, state } => self.record_event(&[
                KeyValue::new("event", "directory_state_changed"),
                KeyValue::new("service", (*service).to_owned()),
                KeyValue::new("state", directory_state_name(*state)),
            ]),
            MetricEvent::CircuitStateChanged {
                scope,
                service,
                state,
            } => self.record_event(&[
                KeyValue::new("event", "circuit_state_changed"),
                KeyValue::new("scope", (*scope).to_owned()),
                KeyValue::new("service", (*service).to_owned()),
                KeyValue::new("state", circuit_state_name(*state)),
            ]),
            MetricEvent::ShutdownFinished {
                runtime,
                outcome,
                duration,
            } => {
                let attributes = [
                    KeyValue::new("event", "shutdown_finished"),
                    KeyValue::new("runtime", (*runtime).to_owned()),
                    KeyValue::new("outcome", outcome_name(*outcome)),
                ];
                self.record_event(&attributes);
                self.record_duration(*duration, &attributes);
            }
        }
    }
}

fn invocation_attributes(
    event: &'static str,
    side: MetricSide,
    protocol: &str,
    service: &str,
    method: &str,
) -> Vec<KeyValue> {
    vec![
        KeyValue::new("event", event),
        KeyValue::new("side", side_name(side)),
        KeyValue::new("protocol", protocol.to_owned()),
        KeyValue::new("service", service.to_owned()),
        KeyValue::new("method", method.to_owned()),
    ]
}

const fn side_name(side: MetricSide) -> &'static str {
    match side {
        MetricSide::Client => "client",
        MetricSide::Server => "server",
    }
}

const fn outcome_name(outcome: MetricOutcome) -> &'static str {
    match outcome {
        MetricOutcome::Success => "success",
        MetricOutcome::Error => "error",
        MetricOutcome::Timeout => "timeout",
        MetricOutcome::Cancelled => "cancelled",
        MetricOutcome::Rejected => "rejected",
    }
}

const fn directory_state_name(state: DirectoryMetricState) -> &'static str {
    match state {
        DirectoryMetricState::Initializing => "initializing",
        DirectoryMetricState::Ready => "ready",
        DirectoryMetricState::Stale => "stale",
        DirectoryMetricState::Unavailable => "unavailable",
        DirectoryMetricState::Closed => "closed",
    }
}

const fn circuit_state_name(state: CircuitState) -> &'static str {
    match state {
        CircuitState::Closed => "closed",
        CircuitState::Open => "open",
        CircuitState::HalfOpen => "half_open",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_accepts_events_without_backend_initialization() {
        let recorder = OpenTelemetryMetricsRecorder::new(&opentelemetry::global::meter(
            "fusen-observability-test",
        ));
        recorder.record(&MetricEvent::AdmissionRejected {
            side: MetricSide::Server,
            reason: "concurrency",
        });
    }
}
