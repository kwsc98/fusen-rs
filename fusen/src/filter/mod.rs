use crate::{error::FusenError, protocol::fusen::context::FusenContext};
use fusen_internal_common::BoxFutureV2;
use std::sync::Arc;

pub trait FusenFilter: Send + Sync {
    fn call<'a>(
        &'a self,
        join_point: ProceedingJoinPoint,
    ) -> BoxFutureV2<'a, Result<FusenContext, FusenError>>;
}

pub struct ProceedingJoinPoint {
    index: usize,
    link: Arc<Vec<Arc<Box<dyn FusenFilter>>>>,
    base_filter: Option<Arc<Box<dyn FusenFilter>>>,
    pub context: FusenContext,
}

impl ProceedingJoinPoint {
    pub fn new(
        link: Arc<Vec<Arc<Box<dyn FusenFilter>>>>,
        base_filter: Arc<Box<dyn FusenFilter>>,
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
                filter.clone().call(self).await
            }
            None => {
                if let Some(ref base_filter) = self.base_filter {
                    base_filter.clone().call(self).await
                } else {
                    Ok(self.into_data())
                }
            }
        }
    }
}
