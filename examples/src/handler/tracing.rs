use fusen_rs::{
    error::FusenError, filter::ProceedingJoinPoint, fusen_procedural_macro::handler,
    handler::aspect::Aspect, protocol::fusen::context::FusenContext,
};
use opentelemetry::propagation::text_map_propagator::TextMapPropagator;
use opentelemetry::trace::TraceContextExt;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing::{Instrument, Span, info_span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[derive(Default)]
pub struct LogAspect {
    trace_context_propagator: TraceContextPropagator,
}

#[handler(id = "LogAspect")]
impl Aspect for LogAspect {
    async fn aroud(&self, mut join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        let context = &mut join_point.context;
        let mut span_context = self
            .trace_context_propagator
            .extract_with_context(&Span::current().context(), &context.request.headers);
        let mut first_span = None;
        let path = &context.request.path.path;
        if !span_context.has_active_span() {
            let span = info_span!("begin_span", path = path);
            span_context = span.context();
            let _ = first_span.insert(span);
        }
        let span = info_span!(
            "trace_span",
            trace_id = span_context.span().span_context().trace_id().to_string(),
            path = path
        );
        span.set_parent(span_context);
        let trace_id = span.context().span().span_context().trace_id().to_string();
        span.set_attribute("trace_id", trace_id.to_owned());
        if !context.request.headers.contains_key("traceparent") {
            self.trace_context_propagator
                .inject_context(&span.context(), &mut context.request.headers);
        };
        let future = async move {
            let context = join_point.proceed().await;
            context
        };
        let result = tokio::spawn(future.instrument(span)).await;
        match result {
            Ok(context) => context,
            Err(error) => Err(FusenError::Error(Box::new(error))),
        }
    }
}
