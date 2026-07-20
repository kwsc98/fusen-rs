use std::time::Instant;

use fusen_rs::{
    error::FusenError, filter::ProceedingJoinPoint, fusen_procedural_macro::handler,
    handler::aspect::Aspect, protocol::fusen::context::FusenContext,
};
use tracing::info;

pub struct LogAspect;

#[handler(id = "LogAspect")]
impl Aspect for LogAspect {
    async fn around(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        let started = Instant::now();
        let request_id = join_point.context.unique_identifier.clone();
        let method = join_point.context.request.path.method.clone();
        let path = join_point.context.request.path.path.clone();
        let result = join_point.proceed().await;
        let status = result
            .as_ref()
            .ok()
            .and_then(|context| context.response.as_ref())
            .map(|response| response.http_status.status)
            .unwrap_or_else(|| {
                result
                    .as_ref()
                    .err()
                    .map(FusenError::status)
                    .unwrap_or(fusen_rs::http::StatusCode::INTERNAL_SERVER_ERROR)
            });
        info!(
            %request_id,
            %method,
            %path,
            status = status.as_u16(),
            elapsed_ms = started.elapsed().as_millis(),
            "request completed"
        );
        result
    }
}
