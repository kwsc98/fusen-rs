use crate::{
    error::FusenError,
    filter::{Middleware, MiddlewareDyn, erase_middleware},
    invocation::InvocationObserver,
    protocol::{
        codec::FusenHttpCodec,
        http::server::{TcpServer, TcpServerConfig},
    },
    server::{
        path::PathCache,
        router::{HttpRouter, RouterContext},
        rpc::{RegisteredRpcService, RouteDispatch},
    },
};
use fusen_contract::{
    ServiceDescriptor, ServiceEndpoint, ServiceRegistration, ServiceWeight, WireProtocol,
};
use fusen_register::Register;
use std::{collections::HashSet, future::Future, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore, time::Instant};

#[allow(missing_docs)]
pub mod path;
#[allow(missing_docs)]
pub mod router;
/// Generated service dispatch contracts.
pub mod rpc;

/// Typed server address, protocol, resource limits, and shutdown deadlines.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Socket address on which the server listens.
    pub bind_addr: SocketAddr,
    /// Externally reachable base URL published to registries.
    pub advertised_base_url: Option<String>,
    /// Maximum bytes accepted for one request body.
    pub max_request_body_bytes: usize,
    /// Maximum number of requests executing concurrently.
    pub max_concurrent_requests: usize,
    /// Maximum number of accepted TCP connections.
    pub max_connections: usize,
    /// Maximum time allowed to read one HTTP/1 request header.
    pub http1_header_read_timeout: Duration,
    /// Maximum concurrent streams advertised for one HTTP/2 connection.
    pub http2_max_concurrent_streams: u32,
    /// Interval between HTTP/2 keep-alive probes.
    pub http2_keep_alive_interval: Duration,
    /// Time allowed for an HTTP/2 keep-alive acknowledgement.
    pub http2_keep_alive_timeout: Duration,
    /// End-to-end server deadline for one request.
    pub request_timeout: Duration,
    /// Maximum time allowed for active connections to drain.
    pub graceful_shutdown_timeout: Duration,
    /// Maximum time allowed for one registry operation.
    pub registry_timeout: Duration,
    /// Wire protocol advertised for registered services.
    pub protocol: WireProtocol,
}

impl ServerConfig {
    /// Creates a configuration with bounded production defaults.
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            bind_addr,
            advertised_base_url: None,
            max_request_body_bytes: 2 * 1024 * 1024,
            max_concurrent_requests: 1024,
            max_connections: 2048,
            http1_header_read_timeout: Duration::from_secs(10),
            http2_max_concurrent_streams: 128,
            http2_keep_alive_interval: Duration::from_secs(30),
            http2_keep_alive_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            graceful_shutdown_timeout: Duration::from_secs(30),
            registry_timeout: Duration::from_secs(5),
            protocol: WireProtocol::Fusen,
        }
    }
}

/// Framework-owned service wrapper used by macro-generated `*Server` types.
#[doc(hidden)]
pub struct ServerService<T> {
    service: T,
    middleware: Vec<Arc<dyn MiddlewareDyn>>,
}

impl<T> ServerService<T> {
    /// Creates a service wrapper.
    #[doc(hidden)]
    pub fn new(service: T) -> Self {
        Self {
            service,
            middleware: Vec::new(),
        }
    }

    /// Appends service-local middleware.
    #[doc(hidden)]
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middleware.push(erase_middleware(middleware));
        self
    }

    /// Erases the generated service for server startup.
    #[doc(hidden)]
    pub fn into_server_service(self) -> PreparedService
    where
        T: RegisteredRpcService + 'static,
    {
        PreparedService {
            service: Arc::new(self.service),
            middleware: self.middleware,
        }
    }
}

/// Conversion implemented by generated service adapters and service-specific wrappers.
#[doc(hidden)]
pub trait IntoServerService: Sized {
    /// Converts one generated service into framework-owned startup state.
    #[doc(hidden)]
    fn into_server_service(self) -> PreparedService
    where
        Self: 'static;
}

/// Erased startup state produced by generated service adapters.
#[doc(hidden)]
pub struct PreparedService {
    service: Arc<dyn RegisteredRpcService>,
    middleware: Vec<Arc<dyn MiddlewareDyn>>,
}

/// Transactional RPC server builder and runtime.
pub struct Server {
    config: ServerConfig,
    bind_error: Option<String>,
    registries: Vec<Arc<dyn Register>>,
    middleware: Vec<Arc<dyn MiddlewareDyn>>,
    services: Vec<PreparedService>,
    observers: Vec<Arc<dyn InvocationObserver>>,
}

impl Server {
    /// Creates a server bound to an IPv4 or IPv6 socket address.
    pub fn bind(address: impl ToString) -> Self {
        let parsed = address.to_string().parse::<SocketAddr>();
        let bind_error = parsed.as_ref().err().map(ToString::to_string);
        let bind_addr = parsed.unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
        Self {
            config: ServerConfig::new(bind_addr),
            bind_error,
            registries: Vec::new(),
            middleware: Vec::new(),
            services: Vec::new(),
            observers: Vec::new(),
        }
    }

    /// Replaces server resource limits and protocol settings.
    pub fn config(mut self, config: ServerConfig) -> Self {
        self.config = config;
        self.bind_error = None;
        self
    }

    /// Adds a registry participating in startup and shutdown transactions.
    pub fn registry(mut self, registry: impl Register + 'static) -> Self {
        self.registries.push(Arc::new(registry));
        self
    }

    /// Appends global provider middleware in execution order.
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middleware.push(erase_middleware(middleware));
        self
    }

    /// Appends a synchronous complete-invocation observer.
    pub fn observer(mut self, observer: impl InvocationObserver + 'static) -> Self {
        self.observers.push(Arc::new(observer));
        self
    }

    /// Registers a generated service implementation or service-specific wrapper.
    pub fn service<T>(mut self, service: T) -> Self
    where
        T: IntoServerService + 'static,
    {
        self.services.push(service.into_server_service());
        self
    }

    /// Runs until Ctrl-C is received and all connections are drained.
    pub async fn run(self) -> Result<(), FusenError> {
        self.run_with_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                tracing::error!(?error, "failed to listen for Ctrl-C");
            }
        })
        .await
    }

    /// Runs with a caller-provided shutdown future.
    pub async fn run_with_shutdown<S>(self, shutdown: S) -> Result<(), FusenError>
    where
        S: Future<Output = ()> + Send,
    {
        self.validate()?;
        let advertised = if self.registries.is_empty() {
            self.config
                .advertised_base_url
                .clone()
                .unwrap_or_else(|| format!("http://{}", self.config.bind_addr))
        } else {
            self.config.advertised_base_url.clone().ok_or_else(|| {
                FusenError::InvalidRequest(
                    "advertised_base_url is required when registration is enabled".into(),
                )
            })?
        };
        let advertised = validate_advertised_url(&advertised)?;
        let mut services = self.services;
        services.sort_by(|left, right| {
            left.service
                .service_descriptor()
                .identity()
                .cmp(right.service.service_descriptor().identity())
        });
        let mut seen = HashSet::new();
        let mut methods = Vec::new();
        let mut dispatches = Vec::new();
        let mut resources = Vec::new();
        for prepared in services {
            let descriptor = prepared.service.service_descriptor();
            if !seen.insert(descriptor.identity()) {
                return Err(FusenError::InvalidRequest(format!(
                    "duplicate service {}",
                    descriptor.identity()
                )));
            }
            let mut middleware =
                Vec::with_capacity(self.middleware.len() + prepared.middleware.len());
            middleware.extend(self.middleware.iter().cloned());
            middleware.extend(prepared.middleware);
            let middleware: Arc<[Arc<dyn MiddlewareDyn>]> = Arc::from(middleware);
            for method in descriptor.methods() {
                methods.push((descriptor, method));
                dispatches.push(RouteDispatch::new(
                    middleware.clone(),
                    prepared.service.clone(),
                ));
            }
            resources.push(service_registration(descriptor, advertised.clone())?);
        }
        let path_cache = PathCache::build(methods)?;
        let listener = TcpListener::bind(self.config.bind_addr)
            .await
            .map_err(|error| FusenError::internal("failed to bind server socket", error))?;

        let mut registered = Vec::new();
        for registry in &self.registries {
            for resource in &resources {
                registered.push((registry.clone(), resource.clone()));
                match tokio::time::timeout(
                    self.config.registry_timeout,
                    registry.register(resource.clone(), self.config.protocol),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        rollback(
                            &registered,
                            self.config.protocol,
                            self.config.registry_timeout,
                            None,
                        )
                        .await;
                        return Err(FusenError::internal("service registration failed", error));
                    }
                    Err(_) => {
                        rollback(
                            &registered,
                            self.config.protocol,
                            self.config.registry_timeout,
                            None,
                        )
                        .await;
                        return Err(FusenError::Timeout(
                            "service registration deadline exceeded".into(),
                        ));
                    }
                }
            }
        }
        let router = HttpRouter {
            context: Arc::new(RouterContext {
                http_codec: FusenHttpCodec::new(self.config.max_request_body_bytes),
                path_cache,
                dispatches: Arc::from(dispatches),
                observers: Arc::from(self.observers),
                concurrency: Arc::new(Semaphore::new(self.config.max_concurrent_requests)),
                request_timeout: self.config.request_timeout,
            }),
        };
        let protocol = self.config.protocol;
        let registry_timeout = self.config.registry_timeout;
        let on_shutdown = move |deadline| async move {
            rollback(&registered, protocol, registry_timeout, Some(deadline)).await;
        };
        let tcp_config = TcpServerConfig {
            max_connections: self.config.max_connections,
            http1_header_read_timeout: self.config.http1_header_read_timeout,
            http2_max_concurrent_streams: self.config.http2_max_concurrent_streams,
            http2_keep_alive_interval: self.config.http2_keep_alive_interval,
            http2_keep_alive_timeout: self.config.http2_keep_alive_timeout,
        };
        TcpServer::run(
            listener,
            router,
            shutdown,
            on_shutdown,
            self.config.graceful_shutdown_timeout,
            tcp_config,
        )
        .await
        .map_err(|error| FusenError::internal("HTTP server failed", error))
    }

    fn validate(&self) -> Result<(), FusenError> {
        if let Some(error) = &self.bind_error {
            return Err(FusenError::InvalidRequest(format!(
                "invalid server bind address: {error}"
            )));
        }
        if self.services.is_empty() {
            return Err(FusenError::InvalidRequest(
                "server must register at least one service".into(),
            ));
        }
        if self.config.max_request_body_bytes == 0
            || self.config.max_concurrent_requests == 0
            || self.config.max_connections == 0
            || self.config.http2_max_concurrent_streams == 0
            || self.config.request_timeout.is_zero()
            || self.config.graceful_shutdown_timeout.is_zero()
            || self.config.registry_timeout.is_zero()
            || self.config.http1_header_read_timeout.is_zero()
            || self.config.http2_keep_alive_interval.is_zero()
            || self.config.http2_keep_alive_timeout.is_zero()
        {
            return Err(FusenError::InvalidRequest(
                "server limits and timeouts must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

fn service_registration(
    descriptor: &'static ServiceDescriptor,
    endpoint: ServiceEndpoint,
) -> Result<Arc<ServiceRegistration>, FusenError> {
    Ok(Arc::new(
        ServiceRegistration::__new(descriptor, endpoint, ServiceWeight::default())
            .map_err(|error| FusenError::InvalidRequest(error.to_string()))?,
    ))
}

fn validate_advertised_url(value: &str) -> Result<ServiceEndpoint, FusenError> {
    value
        .parse::<ServiceEndpoint>()
        .map_err(|error| FusenError::InvalidRequest(error.to_string()))
}

async fn rollback(
    entries: &[(Arc<dyn Register>, Arc<ServiceRegistration>)],
    protocol: WireProtocol,
    operation_timeout: Duration,
    deadline: Option<Instant>,
) {
    for (registry, resource) in entries.iter().rev() {
        let remaining = deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(operation_timeout);
        let timeout = operation_timeout.min(remaining);
        if timeout.is_zero() {
            tracing::error!(service = %resource.selector().service_id(), "service deregistration skipped after shutdown deadline");
            break;
        }
        match tokio::time::timeout(timeout, registry.deregister(resource.clone(), protocol)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::error!(?error, service = %resource.selector().service_id(), "service deregistration failed");
            }
            Err(_) => {
                tracing::error!(service = %resource.selector().service_id(), "service deregistration timed out");
            }
        }
    }
}
