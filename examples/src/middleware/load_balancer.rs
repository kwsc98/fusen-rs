//! Custom load balancer example.

use fusen_rs::{InstanceSnapshot, LoadBalancer, RpcCategory, RpcContext, RpcError};
use rand::RngExt;
use tracing::debug;

/// Uniformly selects one available service instance.
pub struct RandomLoadBalancer;

impl LoadBalancer for RandomLoadBalancer {
    fn select(
        &self,
        context: &RpcContext,
        instances: &InstanceSnapshot,
    ) -> Result<usize, RpcError> {
        debug!(
            request_id = %context.request_id(),
            service = context.interface().identity(),
            method = context.method().fusen_identity(),
            "load balancing request"
        );
        if instances.is_empty() {
            return Err(RpcError::new(
                RpcCategory::Unavailable,
                "no_instances",
                "no healthy service instances",
            )
            .expect("the static error code is valid"));
        }
        Ok(rand::rng().random_range(0..instances.len()))
    }
}
