use super::{
    invocation::{EndpointSource, ServiceClient, ServiceClientInner},
    runtime::ClientRuntime,
};
use crate::{
    ClientError, ClientErrorKind, InstanceRouter, LoadBalancer, Middleware, WeightedRandom,
    middleware::erase_middleware,
};
use fusen_contract::{ServiceDescriptor, ServiceEndpoint, WireProtocol};
use std::{marker::PhantomData, sync::Arc, sync::atomic::Ordering};

enum EndpointMode {
    Unset,
    Direct(Result<ServiceEndpoint, String>),
    Discovery,
}

type DescriptorFn = fn() -> Result<&'static ServiceDescriptor, String>;

/// Generic builder for a macro-generated interface client.
pub struct ClientBuilder<C> {
    runtime: ClientRuntime,
    descriptor: DescriptorFn,
    create: fn(ServiceClient) -> C,
    endpoint: EndpointMode,
    protocol: WireProtocol,
    middleware: Vec<Arc<dyn Middleware>>,
    attempt_middleware: Vec<Arc<dyn Middleware>>,
    routers: Vec<Arc<dyn InstanceRouter>>,
    load_balancer: Arc<dyn LoadBalancer>,
    marker: PhantomData<fn() -> C>,
}

impl<C> ClientBuilder<C> {
    /// Creates a generated-interface client builder.
    #[doc(hidden)]
    pub fn new(
        runtime: &ClientRuntime,
        descriptor: DescriptorFn,
        create: fn(ServiceClient) -> C,
    ) -> Self {
        Self {
            runtime: runtime.clone(),
            descriptor,
            create,
            endpoint: EndpointMode::Unset,
            protocol: WireProtocol::FusenV1,
            middleware: Vec::new(),
            attempt_middleware: Vec::new(),
            routers: Vec::new(),
            load_balancer: Arc::new(WeightedRandom),
            marker: PhantomData,
        }
    }

    /// Uses one explicitly configured HTTP or HTTPS endpoint.
    pub fn direct(mut self, endpoint: impl AsRef<str>) -> Self {
        self.endpoint = EndpointMode::Direct(
            endpoint
                .as_ref()
                .parse::<ServiceEndpoint>()
                .map_err(|error| error.to_string()),
        );
        self
    }

    /// Resolves providers through the runtime registry.
    pub fn discover(mut self) -> Self {
        self.endpoint = EndpointMode::Discovery;
        self
    }

    /// Selects the versioned wire protocol.
    pub fn protocol(mut self, protocol: WireProtocol) -> Self {
        self.protocol = protocol;
        self
    }

    /// Appends interface-local logical-call middleware.
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middleware.push(erase_middleware(middleware));
        self
    }

    /// Appends interface-local physical-attempt middleware.
    pub fn attempt_middleware(mut self, middleware: impl Middleware) -> Self {
        self.attempt_middleware.push(erase_middleware(middleware));
        self
    }

    /// Appends an instance router.
    pub fn instance_router(mut self, router: impl InstanceRouter) -> Self {
        self.routers.push(Arc::new(router));
        self
    }

    /// Replaces the weighted-random load balancer.
    pub fn load_balancer(mut self, load_balancer: impl LoadBalancer) -> Self {
        self.load_balancer = Arc::new(load_balancer);
        self
    }

    /// Validates the interface before activating discovery or returning a ready client.
    pub async fn connect(self) -> Result<C, ClientError> {
        if self.runtime.inner.state.load(Ordering::Acquire) != super::runtime::CLIENT_RUNNING {
            return Err(ClientError::message(
                ClientErrorKind::Closed,
                "client runtime is draining or closed",
            ));
        }
        let interface = (self.descriptor)().map_err(|reason| {
            ClientError::message(
                ClientErrorKind::Connect,
                format!("invalid interface schema: {reason}"),
            )
        })?;
        if !interface.supported_protocols().contains(self.protocol) {
            return Err(ClientError::message(
                ClientErrorKind::Connect,
                format!(
                    "interface {} does not implement {}",
                    interface.identity(),
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
                    .acquire(interface.selector().clone(), self.protocol)
                    .await?;
                EndpointSource::Discovery(directory)
            }
            EndpointMode::Unset => {
                return Err(ClientError::message(
                    ClientErrorKind::Connect,
                    "client must select direct() or discover()",
                ));
            }
        };
        let mut middleware =
            Vec::with_capacity(self.runtime.inner.middleware.len() + self.middleware.len());
        middleware.extend(self.runtime.inner.middleware.iter().cloned());
        middleware.extend(self.middleware);
        let mut attempt_middleware = Vec::with_capacity(
            self.runtime.inner.attempt_middleware.len() + self.attempt_middleware.len(),
        );
        attempt_middleware.extend(self.runtime.inner.attempt_middleware.iter().cloned());
        attempt_middleware.extend(self.attempt_middleware);
        let client = ServiceClient {
            inner: Arc::new(ServiceClientInner {
                runtime: self.runtime.inner,
                service: interface,
                protocol: self.protocol,
                source,
                middleware: Arc::from(middleware),
                attempt_middleware: Arc::from(attempt_middleware),
                routers: Arc::from(self.routers),
                load_balancer: self.load_balancer,
            }),
        };
        Ok((self.create)(client))
    }
}
