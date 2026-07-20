use std::time::Instant;

use fusen_rs::{
    error::FusenError, filter::ProceedingJoinPoint, fusen_procedural_macro::handler,
    handler::aspect::Aspect, protocol::fusen::context::FusenContext,
};
use tracing::debug;

pub struct TimeAspect;

#[handler(id = "TimeAspect")]
impl Aspect for TimeAspect {
    async fn around(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        let start_time = Instant::now();
        debug!("request timer started");
        let context = join_point.proceed().await;
        debug!(
            elapsed_ms = start_time.elapsed().as_millis(),
            "request timer stopped"
        );
        context
    }
}
