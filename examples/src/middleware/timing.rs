//! Invocation timing middleware example.

use fusen_rs::{Middleware, MiddlewareFuture, Next, RpcContext};
use std::time::Instant;
use tracing::debug;

/// Records the elapsed duration of one logical invocation.
pub struct TimingMiddleware;

impl Middleware for TimingMiddleware {
    fn call<'a>(&'a self, context: RpcContext, next: Next<'a>) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            let started = Instant::now();
            let result = next.run(context).await;
            debug!(elapsed_ms = started.elapsed().as_millis(), "request timed");
            result
        })
    }
}
