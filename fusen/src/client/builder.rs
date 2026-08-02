use super::{
    invocation::{EndpointSource, ServiceClient, ServiceClientInner},
    runtime::ClientRuntime,
};
use crate::{
    ClientError, ClientErrorKind, InstanceRouter, Interceptor, LoadBalancer, WeightedRandom,
    interceptor::erase_interceptor, wire::validate_json_service,
};
use fusen_contract::{
    ContractError, EndpointCapabilities, HTTP_JSON_V1, HttpBindingId, HttpVersionPolicy,
    ServiceDescriptor, ServiceEndpoint,
};
use std::{marker::PhantomData, sync::Arc, sync::atomic::Ordering};

enum EndpointMode {
    Unset,
    Direct(Result<ServiceEndpoint, ContractError>),
    Discovery,
}

type DescriptorFn = fn() -> Result<&'static ServiceDescriptor, String>;

/// Generic builder for a macro-generated interface client.
pub struct ClientBuilder<C> {
    runtime: ClientRuntime,
    descriptor: DescriptorFn,
    create: fn(ServiceClient) -> C,
    endpoint: EndpointMode,
    binding_id: HttpBindingId,
    http_version_policy: HttpVersionPolicy,
    direct_capabilities: Option<EndpointCapabilities>,
    interceptor: Vec<Arc<dyn Interceptor>>,
    attempt_interceptor: Vec<Arc<dyn Interceptor>>,
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
            binding_id: HttpBindingId::default(),
            http_version_policy: HttpVersionPolicy::Auto,
            direct_capabilities: None,
            interceptor: Vec::new(),
            attempt_interceptor: Vec::new(),
            routers: Vec::new(),
            load_balancer: Arc::new(WeightedRandom),
            marker: PhantomData,
        }
    }

    /// Uses one explicitly configured HTTP or HTTPS endpoint.
    pub fn direct(mut self, endpoint: impl AsRef<str>) -> Self {
        self.endpoint = EndpointMode::Direct(endpoint.as_ref().parse::<ServiceEndpoint>());
        self
    }

    /// Resolves providers through the runtime registry.
    pub fn discover(mut self) -> Self {
        self.endpoint = EndpointMode::Discovery;
        self
    }

    /// Selects a registered HTTP request and response binding.
    pub fn binding(mut self, binding_id: HttpBindingId) -> Self {
        self.binding_id = binding_id;
        self
    }

    /// Selects an HTTP transport-version policy independently from the binding.
    pub fn http_version_policy(mut self, policy: HttpVersionPolicy) -> Self {
        self.http_version_policy = policy;
        self
    }

    /// Attaches known capabilities to a direct endpoint, enabling negotiated controls.
    pub fn direct_capabilities(mut self, capabilities: EndpointCapabilities) -> Self {
        self.direct_capabilities = Some(capabilities);
        self
    }

    /// Appends interface-local logical-call interceptor.
    pub fn interceptor(mut self, interceptor: impl Interceptor) -> Self {
        self.interceptor.push(erase_interceptor(interceptor));
        self
    }

    /// Appends interface-local physical-attempt interceptor.
    pub fn attempt_interceptor(mut self, interceptor: impl Interceptor) -> Self {
        self.attempt_interceptor
            .push(erase_interceptor(interceptor));
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
            return Err(ClientError::from_message(
                ClientErrorKind::Closed,
                "client runtime is draining or closed",
            ));
        }
        let interface = (self.descriptor)().map_err(|reason| {
            ClientError::from_message(
                ClientErrorKind::Connect,
                format!("invalid interface schema: {reason}"),
            )
        })?;
        let binding = self
            .runtime
            .inner
            .http_bindings
            .get(&self.binding_id)
            .cloned()
            .ok_or_else(|| {
                ClientError::from_message(
                    ClientErrorKind::Connect,
                    format!("HTTP binding {} is not registered", self.binding_id),
                )
            })?;
        if self.binding_id.as_str() == HTTP_JSON_V1 {
            validate_json_service(interface).map_err(|reason| {
                ClientError::from_message(
                    ClientErrorKind::Connect,
                    format!("invalid http-json-v1 interface: {reason}"),
                )
            })?;
        }
        let source = match self.endpoint {
            EndpointMode::Direct(endpoint) => {
                if self
                    .direct_capabilities
                    .as_ref()
                    .is_some_and(|capabilities| !capabilities.supports_binding(&self.binding_id))
                {
                    return Err(ClientError::from_message(
                        ClientErrorKind::Connect,
                        format!(
                            "direct endpoint does not support HTTP binding {}",
                            self.binding_id
                        ),
                    ));
                }
                EndpointSource::Direct {
                    endpoint: endpoint.map_err(|error| {
                        ClientError::with_source(
                            ClientErrorKind::Connect,
                            "invalid direct service endpoint",
                            error,
                        )
                    })?,
                    capabilities: self.direct_capabilities,
                }
            }
            EndpointMode::Discovery => {
                let manager = self.runtime.inner.subscriptions.as_ref().ok_or_else(|| {
                    ClientError::from_message(
                        ClientErrorKind::Discovery,
                        "discover() requires a registry on ClientRuntime",
                    )
                })?;
                let directory = manager.acquire(interface.selector().clone()).await?;
                EndpointSource::Discovery(directory)
            }
            EndpointMode::Unset => {
                return Err(ClientError::from_message(
                    ClientErrorKind::Connect,
                    "client must select direct() or discover()",
                ));
            }
        };
        let mut interceptor =
            Vec::with_capacity(self.runtime.inner.interceptor.len() + self.interceptor.len());
        interceptor.extend(self.runtime.inner.interceptor.iter().cloned());
        interceptor.extend(self.interceptor);
        let mut attempt_interceptor = Vec::with_capacity(
            self.runtime.inner.attempt_interceptor.len() + self.attempt_interceptor.len(),
        );
        attempt_interceptor.extend(self.runtime.inner.attempt_interceptor.iter().cloned());
        attempt_interceptor.extend(self.attempt_interceptor);
        let client = ServiceClient {
            inner: Arc::new(ServiceClientInner {
                runtime: self.runtime.inner,
                service: interface,
                binding_id: self.binding_id,
                binding,
                http_version_policy: self.http_version_policy,
                source,
                interceptor: Arc::from(interceptor),
                attempt_interceptor: Arc::from(attempt_interceptor),
                routers: Arc::from(self.routers),
                load_balancer: self.load_balancer,
            }),
        };
        Ok((self.create)(client))
    }
}
