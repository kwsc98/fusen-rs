//! Logical RPC span middleware example.

use fusen_rs::{Middleware, MiddlewareFuture, Next, RpcContext};
use tracing::{Instrument, info_span};

/// Instruments each logical invocation with bounded service metadata.
#[derive(Default)]
pub struct TracingMiddleware;

impl Middleware for TracingMiddleware {
    fn call<'a>(&'a self, context: RpcContext, next: Next<'a>) -> MiddlewareFuture<'a> {
        let span = info_span!(
            "rpc",
            request_id = %context.request_id(),
            service = context.interface().identity(),
            method = context.method().fusen_identity(),
            protocol = context.protocol().as_str(),
        );
        Box::pin(next.run(context).instrument(span))
    }
}
