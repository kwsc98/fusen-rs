use super::{
    invocation::{EndpointSource, ServiceClient, ServiceClientInner},
    runtime::ClientRuntime,
};
use crate::{
    ClientError, ClientErrorKind, LoadBalancer, Middleware, Router, WeightedRandom,
    middleware::{MiddlewareDyn, erase_middleware},
};
use fusen_contract::{ServiceDescriptor, ServiceEndpoint, WireProtocol};
use std::sync::{Arc, atomic::Ordering};

enum EndpointMode {
    Unset,
    Direct(Result<ServiceEndpoint, String>),
    Discovery,
}

/// Framework client builder wrapped by every macro-generated service builder.
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
    /// Creates a service-specific builder.
    #[doc(hidden)]
    pub fn new(runtime: &ClientRuntime, service: &'static ServiceDescriptor) -> Self {
        Self {
            runtime: runtime.clone(),
            service,
            endpoint: EndpointMode::Unset,
            protocol: WireProtocol::FusenV1,
            middleware: Vec::new(),
            routers: Vec::new(),
            load_balancer: Arc::new(WeightedRandom),
        }
    }

    /// Selects a canonical HTTP or HTTPS endpoint.
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

    /// Selects one explicit wire protocol.
    pub fn protocol(mut self, protocol: WireProtocol) -> Self {
        self.protocol = protocol;
        self
    }

    /// Appends service-local logical middleware.
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middleware.push(erase_middleware(middleware));
        self
    }

    /// Appends an instance router.
    pub fn router(mut self, router: impl Router) -> Self {
        self.routers.push(Arc::new(router));
        self
    }

    /// Replaces the weighted-random load balancer.
    pub fn load_balancer(mut self, load_balancer: impl LoadBalancer) -> Self {
        self.load_balancer = Arc::new(load_balancer);
        self
    }

    /// Activates discovery or validates a direct endpoint.
    pub async fn connect(self) -> Result<ServiceClient, ClientError> {
        if self.runtime.inner.state.load(Ordering::Acquire) != super::runtime::CLIENT_RUNNING {
            return Err(ClientError::message(
                ClientErrorKind::Closed,
                "client runtime is draining or closed",
            ));
        }
        if !self.service.supported_protocols().contains(self.protocol) {
            return Err(ClientError::message(
                ClientErrorKind::Connect,
                format!(
                    "service {} does not implement {}",
                    self.service.identity(),
                    self.protocol
                ),
            ));
        }
        let source = match self.endpoint {
            EndpointMode::Direct(endpoint) => EndpointSource::Direct(
                endpoint.map_err(|error| ClientError::message(ClientErrorKind::Connect, error))?,
            ),
            EndpointMode::Discovery => {
                let manager = self.runtime.inner.subscriptions.as_ref().ok_or_else(|| {
                    ClientError::message(
                        ClientErrorKind::Discovery,
                        "discover() requires a registry on ClientRuntime",
                    )
                })?;
                let directory = manager
                    .acquire(self.service.selector().clone(), self.protocol)
                    .await?;
                EndpointSource::Discovery(directory)
            }
            EndpointMode::Unset => {
                return Err(ClientError::message(
                    ClientErrorKind::Connect,
                    "generated client must select direct() or discover()",
                ));
            }
        };
        let mut middleware =
            Vec::with_capacity(self.runtime.inner.middleware.len() + self.middleware.len());
        middleware.extend(self.runtime.inner.middleware.iter().cloned());
        middleware.extend(self.middleware);
        Ok(ServiceClient {
            inner: Arc::new(ServiceClientInner {
                runtime: self.runtime.inner,
                service: self.service,
                protocol: self.protocol,
                source,
                middleware: Arc::from(middleware),
                routers: Arc::from(self.routers),
                load_balancer: self.load_balancer,
            }),
        })
    }
}
