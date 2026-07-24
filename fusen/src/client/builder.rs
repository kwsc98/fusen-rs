use super::{
    cluster::{LoadBalancer, Router, WeightedRandom},
    invocation::ServiceClient,
    runtime::ClientRuntime,
    subscription::SubscriptionKey,
};
use crate::{
    error::FusenError,
    filter::{Middleware, MiddlewareDyn, erase_middleware},
    protocol::fusen::service::ParameterSource,
};
use fusen_contract::{
    ServiceDescriptor, ServiceEndpoint, ServiceInstance, ServiceWeight, WireProtocol,
};
use fusen_register::directory::Directory;
use std::sync::{Arc, atomic::Ordering};

enum EndpointMode {
    Unset,
    Direct(Result<ServiceEndpoint, String>),
    Discovery,
}

/// Framework builder wrapped by each macro-generated service-specific client builder.
#[doc(hidden)]
pub struct ServiceClientBuilder {
    runtime: ClientRuntime,
    service: &'static ServiceDescriptor,
    endpoint: EndpointMode,
    protocol: WireProtocol,
    middleware: Vec<Arc<dyn MiddlewareDyn>>,
    routers: Vec<Arc<dyn Router>>,
    load_balancer: Arc<dyn LoadBalancer>,
}

impl ServiceClientBuilder {
    pub(super) fn new(runtime: ClientRuntime, service: &'static ServiceDescriptor) -> Self {
        Self {
            runtime,
            service,
            endpoint: EndpointMode::Unset,
            protocol: WireProtocol::Fusen,
            middleware: Vec::new(),
            routers: Vec::new(),
            load_balancer: Arc::new(WeightedRandom),
        }
    }

    /// Selects one validated direct HTTP endpoint.
    pub fn direct(mut self, endpoint: impl AsRef<str>) -> Self {
        self.endpoint = EndpointMode::Direct(
            endpoint
                .as_ref()
                .parse::<ServiceEndpoint>()
                .map_err(|error| error.to_string()),
        );
        self
    }

    /// Selects registry-backed discovery.
    pub fn discover(mut self) -> Self {
        self.endpoint = EndpointMode::Discovery;
        self
    }

    /// Selects the unchanged Fusen or SpringCloud JSON wire behavior.
    pub fn protocol(mut self, protocol: WireProtocol) -> Self {
        self.protocol = protocol;
        self
    }

    /// Appends service-local middleware after runtime-global middleware.
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middleware.push(erase_middleware(middleware));
        self
    }

    /// Appends one router in execution order.
    pub fn router(mut self, router: impl Router) -> Self {
        self.routers.push(Arc::new(router));
        self
    }

    /// Replaces the default weighted-random load balancer.
    pub fn load_balancer(mut self, load_balancer: impl LoadBalancer) -> Self {
        self.load_balancer = Arc::new(load_balancer);
        self
    }

    /// Connects the generated client and initializes discovery when configured.
    pub async fn connect(self) -> Result<Arc<ServiceClient>, FusenError> {
        if self.runtime.inner.closed.load(Ordering::Acquire) {
            return Err(FusenError::ServiceUnavailable(
                "client runtime is shut down".into(),
            ));
        }
        if self.protocol == WireProtocol::SpringCloud
            && self.service.methods().iter().any(|method| {
                method
                    .parameters()
                    .iter()
                    .filter(|parameter| parameter.source() == ParameterSource::Body)
                    .count()
                    > 1
            })
        {
            return Err(FusenError::InvalidRequest(
                "SpringCloud services support at most one body parameter per method".into(),
            ));
        }
        let selector = self.service.selector().clone();
        let (directory, subscription_lease) = match self.endpoint {
            EndpointMode::Direct(endpoint) => {
                let endpoint = endpoint.map_err(FusenError::InvalidRequest)?;
                (
                    Directory::fixed(vec![ServiceInstance::new(
                        endpoint,
                        ServiceWeight::default(),
                    )]),
                    None,
                )
            }
            EndpointMode::Discovery => {
                let registry = self.runtime.inner.registry.as_ref().ok_or_else(|| {
                    FusenError::ServiceUnavailable("discovery requires a client registry".into())
                })?;
                let lease = self
                    .runtime
                    .inner
                    .subscriptions
                    .acquire(
                        SubscriptionKey::new(selector, self.protocol),
                        registry.clone(),
                        self.runtime.inner.config.discovery_timeout,
                    )
                    .await?;
                (lease.directory().clone(), Some(lease))
            }
            EndpointMode::Unset => {
                return Err(FusenError::InvalidRequest(
                    "client endpoint must use direct() or discover()".into(),
                ));
            }
        };
        if self.runtime.inner.closed.load(Ordering::Acquire) {
            drop(subscription_lease);
            return Err(FusenError::ServiceUnavailable(
                "client runtime is shut down".into(),
            ));
        }
        let mut middleware =
            Vec::with_capacity(self.runtime.inner.middleware.len() + self.middleware.len());
        middleware.extend(self.runtime.inner.middleware.iter().cloned());
        middleware.extend(self.middleware);
        Ok(Arc::new(ServiceClient {
            runtime: self.runtime.inner,
            service: self.service,
            protocol: self.protocol,
            directory,
            _subscription_lease: subscription_lease,
            middleware: Arc::from(middleware),
            routers: Arc::from(self.routers),
            load_balancer: self.load_balancer,
        }))
    }
}
