use crate::{RpcCategory, RpcContext, RpcError};
use fusen_contract::ServiceInstance;
use rand::RngExt;
use std::{ops::Deref, sync::Arc};

/// Immutable provider set passed through routers and load balancers.
#[derive(Clone, Debug)]
pub struct InstanceSnapshot(Arc<[ServiceInstance]>);

impl InstanceSnapshot {
    /// Creates a snapshot from owned instances.
    pub fn new(instances: Vec<ServiceInstance>) -> Self {
        Self(Arc::from(instances))
    }

    /// Returns whether two snapshots share the same allocation.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for InstanceSnapshot {
    type Target = [ServiceInstance];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Filters or reorders the latest discovery snapshot before endpoint selection.
pub trait Router: Send + Sync + 'static {
    /// Returns the eligible snapshot for this attempt.
    fn route(
        &self,
        context: &RpcContext,
        instances: InstanceSnapshot,
    ) -> Result<InstanceSnapshot, RpcError>;
}

/// Selects one endpoint index from a routed snapshot.
pub trait LoadBalancer: Send + Sync + 'static {
    /// Returns a valid index into `instances`.
    fn select(&self, context: &RpcContext, instances: &InstanceSnapshot)
    -> Result<usize, RpcError>;
}

/// Built-in weighted-random load balancer.
#[derive(Clone, Copy, Debug, Default)]
pub struct WeightedRandom;

impl LoadBalancer for WeightedRandom {
    fn select(
        &self,
        _context: &RpcContext,
        instances: &InstanceSnapshot,
    ) -> Result<usize, RpcError> {
        if instances.is_empty() {
            return Err(no_instances());
        }
        let total = instances
            .iter()
            .map(|instance| instance.weight().get())
            .sum::<f64>();
        if !total.is_finite() || total <= 0.0 {
            return Err(no_instances());
        }
        let mut target = rand::rng().random_range(0.0..total);
        for (index, instance) in instances.iter().enumerate() {
            if target < instance.weight().get() {
                return Ok(index);
            }
            target -= instance.weight().get();
        }
        Ok(instances.len() - 1)
    }
}

pub(crate) fn no_instances() -> RpcError {
    RpcError::framework(
        RpcCategory::Unavailable,
        "no_instances",
        "no eligible service instances",
    )
}
