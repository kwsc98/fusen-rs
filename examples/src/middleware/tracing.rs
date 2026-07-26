//! Logical RPC span middleware example.

use fusen_rs::{Middleware, Next, RpcContext, RpcResult};
use tracing::{Instrument, info_span};

/// Instruments each logical invocation with bounded service metadata.
#[derive(Default)]
pub struct TracingMiddleware;

impl Middleware for TracingMiddleware {
    async fn handle<'a>(&'a self, context: RpcContext, next: Next<'a>) -> RpcResult {
        let span = info_span!(
            "rpc",
            request_id = %context.request_id(),
            service = context.service().identity(),
            method = context.method().fusen_identity(),
            protocol = context.protocol().as_str(),
        );
        next.run(context).instrument(span).await
    }
}
