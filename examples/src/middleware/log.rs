//! Minimal application-owned tracing backend and metrics sink.

use fusen_observability::{MetricEvent, MetricsRecorder};
use tracing::info;
use tracing_subscriber::EnvFilter;

/// Logs low-cardinality runtime metric events through `tracing`.
#[derive(Clone, Copy, Debug, Default)]
pub struct LogMetricsRecorder;

impl MetricsRecorder for LogMetricsRecorder {
    fn record(&self, event: &MetricEvent<'_>) {
        info!(?event, "runtime metric");
    }
}

/// Installs a process-wide formatting subscriber with an environment override.
pub fn init_tracing(default_filter: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
