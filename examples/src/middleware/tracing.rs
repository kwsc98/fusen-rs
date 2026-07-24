use fusen_rs::{Middleware, Next, RpcContext, RpcResult};
use opentelemetry::propagation::{Extractor, Injector, text_map_propagator::TextMapPropagator};
use opentelemetry::trace::TraceContextExt;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing::{Instrument, Span, info_span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

struct HeaderExtractor<'a>(&'a http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(http::HeaderName::as_str).collect()
    }
}

struct HeaderInjector<'a>(&'a mut http::HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            http::HeaderName::from_bytes(key.as_bytes()),
            http::HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}

#[derive(Default)]
pub struct TracingMiddleware {
    propagator: TraceContextPropagator,
}

impl Middleware for TracingMiddleware {
    async fn handle<'a>(&'a self, mut context: RpcContext, next: Next<'a>) -> RpcResult {
        let mut parent = self.propagator.extract_with_context(
            &Span::current().context(),
            &HeaderExtractor(context.headers()),
        );
        let mut root_span = None;
        if !parent.has_active_span() {
            let span = info_span!("begin_span");
            parent = span.context();
            let _ = root_span.insert(span);
        }
        let span = info_span!(
            "rpc",
            request_id = %context.request_id(),
            service = %context.service(),
            method = %context.method(),
        );
        let _ = span.set_parent(parent);
        if !context.headers().contains_key("traceparent") {
            self.propagator
                .inject_context(&span.context(), &mut HeaderInjector(context.headers_mut()));
        }
        next.run(context).instrument(span).await
    }
}
