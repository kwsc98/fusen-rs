use fusen_internal_common::BoxFuture;

use crate::{
    error::FusenError,
    filter::{FusenFilter, ProceedingJoinPoint},
    protocol::fusen::context::FusenContext,
};

#[allow(async_fn_in_trait)]
pub trait Aspect {
    async fn aroud(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError>;
}

pub struct DefaultAspect;

impl FusenFilter for DefaultAspect {
    fn call(
        &'static self,
        join_point: ProceedingJoinPoint,
    ) -> BoxFuture<Result<FusenContext, FusenError>> {
        Box::pin(async move { join_point.proceed().await })
    }
}
