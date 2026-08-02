pub use crate::{
    context::Context,
    error::{Error, ErrorCategory},
    resilience::{FailureClass, RetryDecision, RetryDecisionContext, RetryPolicy},
};
pub use fusen_contract::ServiceInstance;
use rand::RngExt;
use std::{ops::Deref, sync::Arc};

/// Immutable provider set passed through instance routers and load balancers.
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

/// Input to one instance-routing decision.
pub struct RouteRequest<'a> {
    context: &'a Context,
    instances: InstanceSnapshot,
}

impl<'a> RouteRequest<'a> {
    pub(crate) fn new(context: &'a Context, instances: InstanceSnapshot) -> Self {
        Self { context, instances }
    }

    /// Returns attempt-scoped service invocation metadata.
    pub const fn context(&self) -> &Context {
        self.context
    }

    /// Returns the current immutable provider snapshot.
    pub const fn instances(&self) -> &InstanceSnapshot {
        &self.instances
    }

    /// Consumes this request and returns the provider snapshot.
    pub fn into_instances(self) -> InstanceSnapshot {
        self.instances
    }
}

/// Filters or reorders a discovery snapshot before endpoint selection.
pub trait InstanceRouter: Send + Sync + 'static {
    /// Returns the eligible snapshot for this attempt.
    fn route(&self, request: RouteRequest<'_>) -> Result<InstanceSnapshot, Error>;
}

impl<T> InstanceRouter for Arc<T>
where
    T: InstanceRouter + ?Sized,
{
    fn route(&self, request: RouteRequest<'_>) -> Result<InstanceSnapshot, Error> {
        (**self).route(request)
    }
}

/// Selects one endpoint index from a routed snapshot.
pub trait LoadBalancer: Send + Sync + 'static {
    /// Returns a valid index into `instances`.
    fn select(&self, context: &Context, instances: &InstanceSnapshot) -> Result<usize, Error>;
}

impl<T> LoadBalancer for Arc<T>
where
    T: LoadBalancer + ?Sized,
{
    fn select(&self, context: &Context, instances: &InstanceSnapshot) -> Result<usize, Error> {
        (**self).select(context, instances)
    }
}

/// Built-in weighted-random load balancer.
#[derive(Clone, Copy, Debug, Default)]
pub struct WeightedRandom;

impl LoadBalancer for WeightedRandom {
    fn select(&self, _context: &Context, instances: &InstanceSnapshot) -> Result<usize, Error> {
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

pub(crate) fn no_instances() -> Error {
    Error::framework(
        ErrorCategory::Unavailable,
        "no_instances",
        "no eligible service instances",
    )
}
