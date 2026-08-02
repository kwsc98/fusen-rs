//! Custom load balancer example.

use fusen_rs::{Context, Error, ErrorCategory, InstanceSnapshot, LoadBalancer};
use rand::RngExt;
use tracing::debug;

/// Uniformly selects one available service instance.
pub struct RandomLoadBalancer;

impl LoadBalancer for RandomLoadBalancer {
    fn select(&self, context: &Context, instances: &InstanceSnapshot) -> Result<usize, Error> {
        debug!(
            request_id = %context.request_id(),
            service = context.interface().identity(),
            method = context.method().invocation_name(),
            "load balancing request"
        );
        if instances.is_empty() {
            return Err(Error::local(
                ErrorCategory::Unavailable,
                "no_instances",
                "no healthy service instances",
            )
            .expect("the static error code is valid"));
        }
        Ok(rand::rng().random_range(0..instances.len()))
    }
}
