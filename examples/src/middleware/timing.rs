use fusen_rs::{Middleware, Next, RpcContext, RpcResult};
use std::time::Instant;
use tracing::debug;

pub struct TimingMiddleware;

impl Middleware for TimingMiddleware {
    async fn handle<'a>(&'a self, context: RpcContext, next: Next<'a>) -> RpcResult {
        let started = Instant::now();
        let result = next.run(context).await;
        debug!(elapsed_ms = started.elapsed().as_millis(), "request timed");
        result
    }
}
