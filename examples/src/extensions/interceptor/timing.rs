//! Invocation timing interceptor example.

use fusen_rs::{Context, Interceptor, InterceptorFuture, Next};
use std::time::Instant;
use tracing::debug;

/// Records the elapsed duration of one logical invocation.
pub struct TimingInterceptor;

impl Interceptor for TimingInterceptor {
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        Box::pin(async move {
            let started = Instant::now();
            let result = next.run(context).await;
            debug!(elapsed_ms = started.elapsed().as_millis(), "request timed");
            result
        })
    }
}
