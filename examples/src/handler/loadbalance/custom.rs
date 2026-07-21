use std::sync::Arc;

use fusen_rs::{
    contract::ServiceInstance, error::FusenError, fusen_procedural_macro::handler,
    handler::loadbalance::LoadBalance, protocol::fusen::context::FusenContext,
};
use rand::Rng;
use tracing::debug;

pub struct CustomLoadBalance;

#[handler(id = "CustomLoadBalance")]
impl LoadBalance for CustomLoadBalance {
    async fn select(
        &self,
        context: &FusenContext,
        invokers: Arc<Vec<Arc<ServiceInstance>>>,
    ) -> Result<Option<Arc<ServiceInstance>>, FusenError> {
        debug!(
            request_id = %context.unique_identifier,
            method = %context.request.path.method,
            path = %context.request.path.path,
            "load balancing request"
        );
        if invokers.is_empty() {
            return Ok(None);
        }
        let mut thread_rng = rand::rng();
        Ok(Some(
            invokers[thread_rng.random_range(0..invokers.len())].clone(),
        ))
    }
}
