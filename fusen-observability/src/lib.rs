#![warn(missing_docs)]
//! Logging, tracing, and optional OpenTelemetry initialization.

use chrono::Local;
#[cfg(feature = "otel")]
use opentelemetry::{StringValue, Value, trace::TracerProvider};
#[cfg(feature = "otel")]
use opentelemetry_otlp::{ExporterBuildError, SpanExporter, WithExportConfig};
#[cfg(feature = "otel")]
use opentelemetry_sdk::{
    Resource, runtime,
    trace::{SdkTracerProvider, span_processor_with_async_runtime},
};
use serde::{Deserialize, Serialize};
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
#[cfg(feature = "otel")]
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
/// Logging output and filtering settings.
pub struct LogConfig {
    /// Default log level used by the generated environment filter.
    pub level: String,
    /// Optional rolling JSON log directory.
    pub path: Option<String>,
    /// Optional OTLP trace exporter endpoint.
    pub endpoint: Option<String>,
    /// Optional environment-filter template; `{level}` is replaced with [`LogConfig::level`].
    pub env_filter: Option<String>,
}

/// Owns background logging workers and the optional trace provider.
pub struct LogWorkGroup {
    _work_guard: Option<WorkerGuard>,
    #[cfg(feature = "otel")]
    trace_provider: Option<SdkTracerProvider>,
}

impl Drop for LogWorkGroup {
    fn drop(&mut self) {
        #[cfg(feature = "otel")]
        if let Some(provider) = &self.trace_provider
            && let Err(error) = provider.shutdown()
        {
            eprintln!("failed to shut down OpenTelemetry provider: {error}");
        }
    }
}

#[cfg(feature = "otel")]
fn trace_provider(endpoint: &str, app_name: &str) -> Result<SdkTracerProvider, ExporterBuildError> {
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;
    Ok(SdkTracerProvider::builder()
        .with_resource(
            Resource::builder()
                .with_service_name(Value::String(StringValue::from(app_name.to_owned())))
                .build(),
        )
        .with_span_processor(
            span_processor_with_async_runtime::BatchSpanProcessor::builder(
                exporter,
                runtime::Tokio,
            )
            .build(),
        )
        .build())
}

/// Installs the process tracing subscriber, returning its background resource guards.
pub fn init_log(app_name: &str, config: LogConfig) -> Option<LogWorkGroup> {
    let filter_source = config
        .env_filter
        .as_deref()
        .map(|value| value.replace("{level}", &config.level))
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| {
            if config.level.is_empty() {
                "info".into()
            } else {
                config.level.clone()
            }
        });
    let filter = EnvFilter::try_new(filter_source).unwrap_or_else(|error| {
        eprintln!("invalid log filter, falling back to info: {error}");
        EnvFilter::new("info")
    });
    let mut guard = None;
    let file_layer = config.path.as_ref().and_then(|path| {
        match RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix(app_name)
            .filename_suffix("log")
            .build(path)
        {
            Ok(appender) => {
                let (writer, worker_guard) = tracing_appender::non_blocking(appender);
                guard = Some(worker_guard);
                Some(
                    tracing_subscriber::fmt::layer()
                        .json()
                        .with_writer(writer)
                        .boxed(),
                )
            }
            Err(error) => {
                eprintln!("failed to initialize log file: {error}");
                None
            }
        }
    });
    let console = tracing_subscriber::fmt::layer().with_timer(LocalTimer);

    #[cfg(feature = "otel")]
    let provider =
        config
            .endpoint
            .as_deref()
            .and_then(|endpoint| match trace_provider(endpoint, app_name) {
                Ok(provider) => Some(provider),
                Err(error) => {
                    eprintln!("failed to initialize OpenTelemetry: {error}");
                    None
                }
            });
    #[cfg(feature = "otel")]
    let initialized = tracing_subscriber::registry()
        .with(filter)
        .with(console)
        .with(file_layer)
        .with(
            provider
                .as_ref()
                .map(|provider| OpenTelemetryLayer::new(provider.tracer(app_name.to_owned()))),
        )
        .try_init();

    #[cfg(not(feature = "otel"))]
    let initialized = {
        if config.endpoint.is_some() {
            eprintln!("OpenTelemetry endpoint ignored because the `otel` feature is disabled");
        }
        tracing_subscriber::registry()
            .with(filter)
            .with(console)
            .with(file_layer)
            .try_init()
    };

    if initialized.is_err() {
        return None;
    }
    Some(LogWorkGroup {
        _work_guard: guard,
        #[cfg(feature = "otel")]
        trace_provider: provider,
    })
}

struct LocalTimer;

impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(
        &self,
        writer: &mut tracing_subscriber::fmt::format::Writer<'_>,
    ) -> std::fmt::Result {
        write!(writer, "{}", Local::now().format("%FT%T%.3f%:z"))
    }
}
