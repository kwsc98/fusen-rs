use crate::{error::FusenError, protocol::fusen::context::FusenContext};
use fusen_internal_common::BoxFuture;
use std::sync::Arc;

pub trait FusenFilter: Send + Sync + 'static {
    fn call(
        &'static self,
        join_point: ProceedingJoinPoint,
    ) -> BoxFuture<Result<FusenContext, FusenError>>;
}

pub struct ProceedingJoinPoint {
    index: usize,
    link: Arc<Vec<&'static dyn FusenFilter>>,
    base_filter: Option<&'static dyn FusenFilter>,
    pub context: FusenContext,
}

impl ProceedingJoinPoint {
    pub fn new(
        link: Arc<Vec<&'static dyn FusenFilter>>,
        base_filter: &'static dyn FusenFilter,
        context: FusenContext,
    ) -> Self {
        Self {
            index: 0,
            link,
            base_filter: Some(base_filter),
            context,
        }
    }
    pub fn into_data(self) -> FusenContext {
        self.context
    }
    pub async fn proceed(mut self) -> Result<FusenContext, FusenError> {
        match self.link.get(self.index) {
            Some(filter) => {
                self.index += 1;
                filter.call(self).await
            }
            None => {
                if let Some(base_filter) = self.base_filter {
                    base_filter.call(self).await
                } else {
                    Ok(self.into_data())
                }
            }
        }
    }
}
