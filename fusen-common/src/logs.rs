use chrono::Local;
use fusen_procedural_macro::Data;
use opentelemetry::{
    trace::{TraceError, TracerProvider},
    StringValue, Value,
};
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{runtime, trace::TracerProvider as Tracer, Resource};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use tracing_subscriber::{layer::SubscriberExt, EnvFilter};

#[derive(Clone, Data, Debug, Default, Serialize, Deserialize)]
pub struct LogConfig {
    pub path: Option<String>,
    pub endpoint: Option<String>,
    pub env_filter: Option<String>,
    pub devmode: Option<bool>,
}

#[derive(Default, Data)]
pub struct LogWorkGroup {
    work_guard: Option<WorkerGuard>,
    tracer_provider: Option<Tracer>,
}

impl Drop for LogWorkGroup {
    fn drop(&mut self) {
        if let Some(tracer_provider) = &self.tracer_provider {
            let _ = tracer_provider.shutdown();
        }
    }
}

fn init_opentelemetry_trace(otlp_url: &str, app_name: &str) -> Result<Tracer, TraceError> {
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(otlp_url)
        .build()?;
    Ok(Tracer::builder()
        .with_resource(Resource::new(vec![opentelemetry::KeyValue::new(
            "service.name",
            Value::String(StringValue::from(app_name.to_owned())),
        )]))
        .with_batch_exporter(exporter, runtime::Tokio)
        .build())
}

pub fn init_log(log_config: &LogConfig, app_name: &str) -> Option<LogWorkGroup> {
    let mut worker_guard = None;
    let mut tracer_guard = None;
    let mut layter_list = vec![];
    let env_filter = || {
        if let Some(env_filter) = &log_config.env_filter {
            EnvFilter::from_str(env_filter).unwrap()
        } else {
            EnvFilter::from_default_env()
        }
    };
    if let Some(path) = &log_config.path {
        let file_appender = RollingFileAppender::new(Rotation::DAILY, path, app_name);
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        let _ = worker_guard.insert(guard);
        let tracing = tracing_subscriber::fmt::layer()
            .with_line_number(true)
            .with_thread_ids(true);
        let json_tracing = tracing.json().with_writer(non_blocking);
        layter_list.push(json_tracing.boxed());
    };
    if log_config.devmode.is_some_and(|e| e) {
        let tracing = tracing_subscriber::fmt::layer()
            .with_line_number(true)
            .with_thread_ids(true);
        layter_list.push(tracing.boxed());
    }
    if let Some(endpoint) = &log_config.endpoint {
        let provider = init_opentelemetry_trace(endpoint, app_name).unwrap();
        let _ = tracer_guard.insert(provider.clone());
        let opentelemetry = OpenTelemetryLayer::new(provider.tracer(app_name.to_owned()));
        layter_list.push(opentelemetry.boxed());
    }
    if layter_list.is_empty() {
        return None;
    }
    let mut layer = layter_list.remove(0);
    for item in layter_list {
        layer = Box::new(layer.and_then(item));
    }
    tracing_subscriber::registry()
        .with(env_filter())
        .with(layer)
        .init();
    Some(
        LogWorkGroup::default()
            .tracer_provider(tracer_guard)
            .work_guard(worker_guard),
    )
}

pub fn get_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn get_trade_id() -> String {
    format!(
        "{}-{}",
        uuid::Uuid::new_v4(),
        Local::now().format("%Y%m%d%H%M%S")
    )
}
