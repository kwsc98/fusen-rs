use fusen_rs::{
    error::FusenError, filter::ProceedingJoinPoint, fusen_procedural_macro::handler,
    handler::aspect::Aspect, protocol::fusen::context::FusenContext,
};
use opentelemetry::propagation::{Extractor, Injector, text_map_propagator::TextMapPropagator};
use opentelemetry::trace::TraceContextExt;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing::{Instrument, Span, info_span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

struct HeaderExtractor<'a>(&'a fusen_rs::http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .map(fusen_rs::http::HeaderName::as_str)
            .collect()
    }
}

struct HeaderInjector<'a>(&'a mut fusen_rs::http::HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            fusen_rs::http::HeaderName::from_bytes(key.as_bytes()),
            fusen_rs::http::HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}

#[derive(Default)]
pub struct TraceAspect {
    trace_context_propagator: TraceContextPropagator,
}

#[handler(id = "TraceAspect")]
impl Aspect for TraceAspect {
    async fn around(
        &self,
        mut join_point: ProceedingJoinPoint,
    ) -> Result<FusenContext, FusenError> {
        let context = &mut join_point.context;
        let mut span_context = self.trace_context_propagator.extract_with_context(
            &Span::current().context(),
            &HeaderExtractor(&context.request.headers),
        );
        let mut first_span = None;
        let path = &context.request.path.path;
        if !span_context.has_active_span() {
            let span = info_span!("begin_span");
            span_context = span.context();
            let _ = first_span.insert(span);
        }
        let span = info_span!(
            "trace_span",
            request_id = %context.unique_identifier,
            method = %context.request.path.method,
            path = path
        );
        let _ = span.set_parent(span_context);
        if !context.request.headers.contains_key("traceparent") {
            self.trace_context_propagator.inject_context(
                &span.context(),
                &mut HeaderInjector(&mut context.request.headers),
            );
        };
        async move { join_point.proceed().await }
            .instrument(span)
            .await
    }
}
