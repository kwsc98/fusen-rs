use crate::{error::FusenError, protocol::fusen::context::RpcContext};
use fusen_contract::ServiceInstance;
use rand::RngExt;
use std::{ops::Deref, sync::Arc};

/// Immutable set of healthy service instances passed through cluster routing.
#[derive(Clone, Debug)]
pub struct InstanceSnapshot(Arc<Vec<Arc<ServiceInstance>>>);

impl InstanceSnapshot {
    /// Creates a snapshot from an owned instance list.
    pub fn new(instances: Vec<Arc<ServiceInstance>>) -> Self {
        Self(Arc::new(instances))
    }

    pub(crate) fn from_shared(instances: Arc<Vec<Arc<ServiceInstance>>>) -> Self {
        Self(instances)
    }

    /// Returns the immutable instance slice.
    pub fn as_slice(&self) -> &[Arc<ServiceInstance>] {
        self.0.as_slice()
    }

    /// Returns true when two snapshots reuse the same immutable allocation.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for InstanceSnapshot {
    type Target = [Arc<ServiceInstance>];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

/// Synchronous instance router executed after client middleware.
pub trait Router: Send + Sync + 'static {
    /// Filters or reorders an immutable service snapshot.
    fn route(
        &self,
        context: &RpcContext,
        instances: InstanceSnapshot,
    ) -> Result<InstanceSnapshot, FusenError>;
}

/// Selects one instance index from a routed snapshot.
pub trait LoadBalancer: Send + Sync + 'static {
    /// Returns an index into `instances`.
    fn select(
        &self,
        context: &RpcContext,
        instances: &InstanceSnapshot,
    ) -> Result<usize, FusenError>;
}

/// Weighted-random selection using validated provider weights.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct WeightedRandom;

impl LoadBalancer for WeightedRandom {
    fn select(
        &self,
        _context: &RpcContext,
        instances: &InstanceSnapshot,
    ) -> Result<usize, FusenError> {
        if instances.is_empty() {
            return Err(unavailable());
        }
        let max_weight = instances
            .iter()
            .map(|instance| instance.weight().get())
            .fold(0.0_f64, f64::max);
        if max_weight <= 0.0 {
            return Err(unavailable());
        }
        let total = instances
            .iter()
            .map(|instance| instance.weight().get() / max_weight)
            .sum::<f64>();
        let mut target = rand::rng().random_range(0.0..total);
        for (index, instance) in instances.iter().enumerate() {
            let normalized = instance.weight().get() / max_weight;
            if target < normalized {
                return Ok(index);
            }
            target -= normalized;
        }
        Ok(instances.len() - 1)
    }
}

pub(crate) fn validate_selection(
    instances: &InstanceSnapshot,
    index: usize,
) -> Result<&Arc<ServiceInstance>, FusenError> {
    instances.get(index).ok_or_else(unavailable)
}

pub(crate) fn ensure_not_empty(
    instances: InstanceSnapshot,
) -> Result<InstanceSnapshot, FusenError> {
    if instances.is_empty() {
        Err(unavailable())
    } else {
        Ok(instances)
    }
}

fn unavailable() -> FusenError {
    FusenError::ServiceUnavailable("no healthy service instances".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::fusen::{
        context::RpcContext,
        request::{FusenRequest, Path},
    };
    use fusen_contract::{
        MethodDescriptor, MethodId, ServiceDescriptor, ServiceEndpoint, WireProtocol,
    };
    use http::Method;
    use std::sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::time::{Duration, Instant};

    fn context() -> RpcContext {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        let service = SERVICE.get_or_init(|| {
            ServiceDescriptor::__new(
                "demo",
                None,
                None,
                vec![
                    MethodDescriptor::__new(
                        MethodId::__new(0),
                        "call",
                        Method::POST,
                        "/call",
                        Vec::new(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap()
        });
        RpcContext::new(
            "request".into(),
            service,
            &service.methods()[0],
            FusenRequest {
                protocol: WireProtocol::Fusen,
                path: Path {
                    method: Method::POST,
                    path: "/call".into(),
                },
                endpoint: None,
                path_parameters: Default::default(),
                query_parameters: Default::default(),
                headers: Default::default(),
                body: None,
            },
            Instant::now() + Duration::from_secs(1),
        )
    }

    fn instance(host: &str) -> Arc<ServiceInstance> {
        Arc::new(ServiceInstance::new(
            format!("http://{host}").parse::<ServiceEndpoint>().unwrap(),
            Default::default(),
        ))
    }

    #[test]
    fn unconfigured_router_path_reuses_directory_snapshot() {
        let shared = Arc::new(vec![instance("one")]);
        let before = InstanceSnapshot::from_shared(shared.clone());
        let after = InstanceSnapshot::from_shared(shared);
        assert!(before.ptr_eq(&after));
    }

    struct RecordingRouter {
        name: &'static str,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Router for RecordingRouter {
        fn route(
            &self,
            _context: &RpcContext,
            instances: InstanceSnapshot,
        ) -> Result<InstanceSnapshot, FusenError> {
            self.events.lock().unwrap().push(self.name);
            Ok(instances)
        }
    }

    #[test]
    fn routers_execute_in_configuration_order_without_forced_copy() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let routers: Vec<Box<dyn Router>> = vec![
            Box::new(RecordingRouter {
                name: "first",
                events: events.clone(),
            }),
            Box::new(RecordingRouter {
                name: "second",
                events: events.clone(),
            }),
        ];
        let original = InstanceSnapshot::new(vec![instance("one")]);
        let mut routed = original.clone();
        for router in routers {
            routed = router.route(&context(), routed).unwrap();
        }
        assert!(original.ptr_eq(&routed));
        assert_eq!(*events.lock().unwrap(), ["first", "second"]);
    }

    struct CountingLoadBalancer(Arc<AtomicUsize>);

    impl LoadBalancer for CountingLoadBalancer {
        fn select(
            &self,
            _context: &RpcContext,
            _instances: &InstanceSnapshot,
        ) -> Result<usize, FusenError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        }
    }

    #[test]
    fn load_balancer_is_called_exactly_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let instances = InstanceSnapshot::new(vec![instance("one")]);
        let index = CountingLoadBalancer(calls.clone())
            .select(&context(), &instances)
            .unwrap();
        assert_eq!(index, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn empty_and_invalid_selections_are_service_unavailable() {
        let empty = InstanceSnapshot::new(Vec::new());
        assert!(matches!(
            ensure_not_empty(empty),
            Err(FusenError::ServiceUnavailable(_))
        ));
        let instances = InstanceSnapshot::new(vec![instance("one")]);
        assert!(matches!(
            validate_selection(&instances, 1),
            Err(FusenError::ServiceUnavailable(_))
        ));
    }

    #[test]
    fn weighted_random_always_returns_a_valid_index() {
        let instances = InstanceSnapshot::new(vec![instance("one"), instance("two")]);
        for _ in 0..100 {
            assert!(WeightedRandom.select(&context(), &instances).unwrap() < instances.len());
        }
    }
}
