use fusen_rs::{
    error::FusenError, filter::ProceedingJoinPoint, fusen_procedural_macro::handler,
    handler::aspect::Aspect, protocol::fusen::context::FusenContext,
};

pub struct LogAspect;

#[handler(id = "LogAspect")]
impl Aspect for LogAspect {
    async fn aroud(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        println!("开始处理 : {:?}", join_point.context.request);
        let context = join_point.proceed().await;
        println!(
            "结束处理 : {:?}",
            context.as_ref().unwrap().response.as_ref().unwrap().body
        );
        context
    }
}
