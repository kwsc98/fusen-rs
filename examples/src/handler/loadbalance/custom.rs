use std::sync::Arc;

use fusen_rs::{
    error::FusenError, fusen_internal_common::resource::service::ServiceResource,
    fusen_procedural_macro::handler, handler::loadbalance::LoadBalance,
    protocol::fusen::context::FusenContext,
};
use rand::Rng;
use tracing::debug;

pub struct CustomLoadBalance;

#[handler(id = "CustomLoadBalance")]
impl LoadBalance for CustomLoadBalance {
    async fn select(
        &self,
        context: &FusenContext,
        invokers: Arc<Vec<Arc<ServiceResource>>>,
    ) -> Result<Option<Arc<ServiceResource>>, FusenError> {
        debug!("Start CustomLoadBalance : {context:?}");
        if invokers.is_empty() {
            return Ok(None);
        }
        let mut thread_rng = rand::rng();
        Ok(Some(
            invokers[thread_rng.random_range(0..invokers.len())].clone(),
        ))
    }
}
