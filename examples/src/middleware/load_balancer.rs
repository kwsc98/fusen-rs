use fusen_rs::{
    RpcContext,
    client::cluster::{InstanceSnapshot, LoadBalancer},
    error::FusenError,
};
use rand::RngExt;
use tracing::debug;

pub struct RandomLoadBalancer;

impl LoadBalancer for RandomLoadBalancer {
    fn select(
        &self,
        context: &RpcContext,
        instances: &InstanceSnapshot,
    ) -> Result<usize, FusenError> {
        debug!(
            request_id = %context.request_id(),
            service = %context.service(),
            method = %context.method(),
            "load balancing request"
        );
        if instances.is_empty() {
            return Err(FusenError::ServiceUnavailable(
                "no healthy service instances".into(),
            ));
        }
        Ok(rand::rng().random_range(0..instances.len()))
    }
}
