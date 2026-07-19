use chrono::Local;
use opentelemetry::{StringValue, Value, trace::TracerProvider};
use opentelemetry_otlp::{ExporterBuildError, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource, runtime,
    trace::{SdkTracerProvider, span_processor_with_async_runtime},
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LogConfig {
    pub level: String,
    pub path: Option<String>,
    pub endpoint: Option<String>,
    pub env_filter: Option<String>,
}

pub struct LogWorkGroup {
    _work_guard: Option<WorkerGuard>,
    trace_provider: Option<SdkTracerProvider>,
}

impl Drop for LogWorkGroup {
    fn drop(&mut self) {
        if let Some(provider) = &self.trace_provider
            && let Err(error) = provider.shutdown()
        {
            eprintln!("failed to shut down OpenTelemetry provider: {error}");
        }
    }
}

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

pub fn init_log(app_name: &str, config: LogConfig) -> Option<LogWorkGroup> {
    let filter_source = config
        .env_filter
        .as_deref()
        .map(|value| value.replace("{level}", &config.level));
    let filter = filter_source
        .as_deref()
        .and_then(|value| EnvFilter::from_str(value).ok())
        .unwrap_or_else(EnvFilter::from_default_env);
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
    let otel_layer = provider
        .as_ref()
        .map(|provider| OpenTelemetryLayer::new(provider.tracer(app_name.to_owned())));
    let console = tracing_subscriber::fmt::layer().with_timer(LocalTimer);
    if tracing_subscriber::registry()
        .with(filter)
        .with(console)
        .with(file_layer)
        .with(otel_layer)
        .try_init()
        .is_err()
    {
        return None;
    }
    Some(LogWorkGroup {
        _work_guard: guard,
        trace_provider: provider,
    })
}

struct LocalTimer;
impl tracing_subscriber::fmt::time::FormatTime for LocalTimer {
    fn format_time(
        &self,
        writer: &mut tracing_subscriber::fmt::format::Writer<'_>,
    ) -> std::fmt::Result {
        write!(writer, "{}", Local::now().format("%FT%T%.3f"))
    }
}
