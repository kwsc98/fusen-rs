use fusen_contract::StaticBoxFuture;

use crate::{
    error::FusenError,
    filter::{FusenFilter, ProceedingJoinPoint},
    protocol::fusen::context::FusenContext,
};

#[allow(async_fn_in_trait)]
pub trait Aspect {
    async fn around(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError>;
}

pub struct DefaultAspect;

impl FusenFilter for DefaultAspect {
    fn call(
        &self,
        join_point: ProceedingJoinPoint,
    ) -> StaticBoxFuture<Result<FusenContext, FusenError>> {
        Box::pin(async move { join_point.proceed().await })
    }
}
