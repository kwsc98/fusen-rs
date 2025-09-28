use std::time::{SystemTime, UNIX_EPOCH};

use fusen_rs::{
    error::FusenError, filter::ProceedingJoinPoint, fusen_procedural_macro::handler,
    handler::aspect::Aspect, protocol::fusen::context::FusenContext,
};
use tracing::debug;

pub struct TimeAspect;

#[handler(id = "TimeAspect")]
impl Aspect for TimeAspect {
    async fn aroud(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        debug!("开始处理时间 : {start_time:?}");
        let context = join_point.proceed().await;
        debug!(
            "结束处理时间 : {:?}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
                - start_time
        );
        context
    }
}
