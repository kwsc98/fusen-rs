use crate::error::FusenError;
use fusen_internal_common::{BoxFuture, resource::service::ServiceResource};
use rand::Rng;
use std::sync::Arc;

#[allow(async_fn_in_trait)]
pub trait LoadBalance {
    async fn select(
        &self,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> Result<Arc<ServiceResource>, FusenError>;
}

pub trait LoadBalance_: Send + Sync {
    fn select_(
        &'static self,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> BoxFuture<Result<Arc<ServiceResource>, FusenError>>;
}

pub struct DefaultLoadBalance;

impl LoadBalance_ for DefaultLoadBalance {
    fn select_(
        &self,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> BoxFuture<Result<Arc<ServiceResource>, FusenError>> {
        Box::pin(async move {
            let mut thread_rng = rand::rng();
            Ok(invokers[thread_rng.random_range(0..invokers.len())].clone())
        })
    }
}
