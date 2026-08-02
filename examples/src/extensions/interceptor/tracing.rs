//! Logical service-invocation span interceptor example.

use fusen_rs::{Context, Interceptor, InterceptorFuture, Next};
use tracing::{Instrument, info_span};

/// Instruments each logical invocation with bounded service metadata.
#[derive(Default)]
pub struct TracingInterceptor;

impl Interceptor for TracingInterceptor {
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        let span = info_span!(
            "invocation",
            request_id = %context.request_id(),
            service = context.interface().identity(),
            method = context.method().invocation_name(),
            http_binding = context.binding_id().as_str(),
            network_protocol_version = ?context.http_version(),
        );
        Box::pin(next.run(context).instrument(span))
    }
}
