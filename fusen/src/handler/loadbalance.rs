use crate::{error::FusenError, protocol::fusen::context::FusenContext};
use fusen_internal_common::{BoxFutureV2, resource::service::ServiceResource};
use rand::Rng;
use std::sync::Arc;

#[allow(async_fn_in_trait)]
pub trait LoadBalance {
    async fn select<'a>(
        &'a self,
        context: &'a FusenContext,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> Result<Option<Arc<ServiceResource>>, FusenError>;
}

pub trait LoadBalance_: Send + Sync {
    fn select_<'a>(
        &'a self,
        context: &'a FusenContext,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> BoxFutureV2<'a, Result<Option<Arc<ServiceResource>>, FusenError>>;
}

pub struct DefaultLoadBalance;

impl LoadBalance_ for DefaultLoadBalance {
    fn select_(
        &'_ self,
        _context: &'_ FusenContext,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> BoxFutureV2<'_, Result<Option<Arc<ServiceResource>>, FusenError>> {
        Box::pin(async move {
            if invokers.is_empty() {
                return Ok(None);
            }
            let mut thread_rng = rand::rng();
            Ok(Some(
                invokers[thread_rng.random_range(0..invokers.len())].clone(),
            ))
        })
    }
}
