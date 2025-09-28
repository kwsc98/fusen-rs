use fusen_rs::{
    error::FusenError, filter::ProceedingJoinPoint, fusen_procedural_macro::handler,
    handler::aspect::Aspect, protocol::fusen::context::FusenContext,
};
use tracing::{debug, info};

pub struct LogAspect;

#[handler(id = "LogAspect")]
impl Aspect for LogAspect {
    async fn aroud(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        info!("开始处理 : {:?}", join_point.context.request);
        let context = join_point.proceed().await;
        debug!(
            "结束处理 : {:?}",
            context.as_ref().unwrap().response.as_ref().unwrap().body
        );
        context
    }
}
