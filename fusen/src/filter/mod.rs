use crate::{error::FusenError, protocol::fusen::context::FusenContext};
use fusen_internal_common::BoxFuture;
use std::collections::LinkedList;

pub trait FusenFilter: Send + Sync + 'static {
    fn call(
        &'static self,
        join_point: ProceedingJoinPoint,
    ) -> BoxFuture<Result<FusenContext, FusenError>>;
}

pub struct ProceedingJoinPoint {
    link: LinkedList<&'static dyn FusenFilter>,
    context: FusenContext,
}

impl ProceedingJoinPoint {
    pub fn new(link: LinkedList<&'static dyn FusenFilter>, context: FusenContext) -> Self {
        Self { link, context }
    }
    pub fn into_data(self) -> FusenContext {
        self.context
    }
    pub async fn proceed(mut self) -> Result<FusenContext, FusenError> {
        match self.link.pop_front() {
            Some(filter) => filter.call(self).await,
            None => Ok(self.into_data()),
        }
    }
}
