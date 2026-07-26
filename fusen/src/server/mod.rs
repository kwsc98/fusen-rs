use crate::{
    error::FusenError,
    filter::{Middleware, MiddlewareDyn, erase_middleware},
    invocation::InvocationObserver,
    protocol::{
        codec::FusenHttpCodec,
        http::server::{ShutdownCompletion, TcpServer, TcpServerConfig, TcpServerOutcome},
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
use fusen_register::{Register, error::RegisterError};
use std::{collections::HashSet, future::Future, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, mpsc, oneshot},
    time::Instant,
};

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
    /// Total deadline shared by service deregistration and connection draining.
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

    /// Runs until SIGINT or SIGTERM on Unix, or Ctrl-C on other platforms.
    pub async fn run(self) -> Result<(), FusenError> {
        self.run_with_shutdown(default_shutdown_signal()).await
    }

    /// Runs with a caller-provided shutdown future as the only shutdown trigger.
    ///
    /// The shutdown future is first polled after listener binding and service
    /// registration complete; startup registry calls remain bounded by
    /// [`ServerConfig::registry_timeout`].
    ///
    /// Once triggered, service deregistration and connection draining share
    /// [`ServerConfig::graceful_shutdown_timeout`]. Dropping this future after
    /// registration starts best-effort background deregistration while the
    /// Tokio runtime remains available.
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

        let cleanup = RegistrationCleanup::spawn(
            self.config.protocol,
            self.config.registry_timeout,
            self.config.graceful_shutdown_timeout,
        );
        for registry in &self.registries {
            for resource in &resources {
                cleanup
                    .track(registry.clone(), resource.clone())
                    .map_err(|error| {
                        FusenError::internal("service registration cleanup failed", error)
                    })?;
                match tokio::time::timeout(
                    self.config.registry_timeout,
                    registry.register(resource.clone(), self.config.protocol),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        if let Err(cleanup_error) = cleanup.cleanup(None).await {
                            tracing::error!(
                                ?cleanup_error,
                                "registration rollback did not complete cleanly"
                            );
                        }
                        return Err(FusenError::internal("service registration failed", error));
                    }
                    Err(_) => {
                        if let Err(cleanup_error) = cleanup.cleanup(None).await {
                            tracing::error!(
                                ?cleanup_error,
                                "registration rollback did not complete cleanly"
                            );
                        }
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
        let on_shutdown = move |deadline| cleanup.cleanup(Some(deadline));
        let tcp_config = TcpServerConfig {
            max_connections: self.config.max_connections,
            http1_header_read_timeout: self.config.http1_header_read_timeout,
            http2_max_concurrent_streams: self.config.http2_max_concurrent_streams,
            http2_keep_alive_interval: self.config.http2_keep_alive_interval,
            http2_keep_alive_timeout: self.config.http2_keep_alive_timeout,
        };
        let outcome = TcpServer::run(
            listener,
            router,
            shutdown,
            on_shutdown,
            self.config.graceful_shutdown_timeout,
            tcp_config,
        )
        .await;
        server_result(outcome)
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

#[cfg(unix)]
async fn default_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = match signal(SignalKind::interrupt()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(
                ?error,
                signal = "SIGINT",
                "failed to install shutdown signal"
            );
            return;
        }
    };
    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(
                ?error,
                signal = "SIGTERM",
                "failed to install shutdown signal"
            );
            return;
        }
    };
    tokio::select! {
        signal = interrupt.recv() => match signal {
            Some(()) => tracing::info!(signal = "SIGINT", "received shutdown signal"),
            None => tracing::error!(signal = "SIGINT", "shutdown signal listener closed"),
        },
        signal = terminate.recv() => match signal {
            Some(()) => tracing::info!(signal = "SIGTERM", "received shutdown signal"),
            None => tracing::error!(signal = "SIGTERM", "shutdown signal listener closed"),
        },
    }
}

#[cfg(not(unix))]
async fn default_shutdown_signal() {
    match tokio::signal::ctrl_c().await {
        Ok(()) => tracing::info!(signal = "Ctrl-C", "received shutdown signal"),
        Err(error) => tracing::error!(
            ?error,
            signal = "Ctrl-C",
            "failed to listen for shutdown signal"
        ),
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

type RegistrationEntry = (Arc<dyn Register>, Arc<ServiceRegistration>);

enum RegistrationCleanupCommand {
    Track(RegistrationEntry),
    Cleanup {
        deadline: Option<Instant>,
        completion: oneshot::Sender<Result<(), RegistrationCleanupError>>,
        waiter_dropped: oneshot::Receiver<()>,
    },
}

struct RegistrationCleanup {
    commands: mpsc::UnboundedSender<RegistrationCleanupCommand>,
}

impl RegistrationCleanup {
    fn spawn(
        protocol: WireProtocol,
        operation_timeout: Duration,
        cancellation_timeout: Duration,
    ) -> Self {
        let (commands, receiver) = mpsc::unbounded_channel();
        tokio::spawn(run_registration_cleanup_worker(
            receiver,
            protocol,
            operation_timeout,
            cancellation_timeout,
        ));
        Self { commands }
    }

    fn track(
        &self,
        registry: Arc<dyn Register>,
        resource: Arc<ServiceRegistration>,
    ) -> Result<(), RegistrationCleanupError> {
        self.commands
            .send(RegistrationCleanupCommand::Track((registry, resource)))
            .map_err(|_| RegistrationCleanupError::WorkerStopped)
    }

    fn cleanup(
        self,
        deadline: Option<Instant>,
    ) -> impl Future<Output = Result<(), RegistrationCleanupError>> + Send {
        let (completion, result) = oneshot::channel();
        // This sender lives in the waiter so cancellation can tighten startup rollback.
        let (waiter_alive, waiter_dropped) = oneshot::channel();
        let sent = self
            .commands
            .send(RegistrationCleanupCommand::Cleanup {
                deadline,
                completion,
                waiter_dropped,
            })
            .map_err(|_| RegistrationCleanupError::WorkerStopped);
        drop(self.commands);
        async move {
            sent?;
            let cleanup = result
                .await
                .map_err(|_| RegistrationCleanupError::WorkerStopped)?;
            drop(waiter_alive);
            cleanup
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum RegistrationCleanupError {
    #[error("service deregistration deadline exceeded")]
    Timeout,
    #[error("service deregistration failed for {service}")]
    Deregister {
        service: String,
        #[source]
        source: RegisterError,
    },
    #[error("registration cleanup worker stopped unexpectedly")]
    WorkerStopped,
}

async fn run_registration_cleanup_worker(
    mut commands: mpsc::UnboundedReceiver<RegistrationCleanupCommand>,
    protocol: WireProtocol,
    operation_timeout: Duration,
    cancellation_timeout: Duration,
) {
    let mut entries = Vec::new();
    while let Some(command) = commands.recv().await {
        match command {
            RegistrationCleanupCommand::Track(entry) => entries.push(entry),
            RegistrationCleanupCommand::Cleanup {
                deadline,
                completion,
                waiter_dropped,
            } => {
                let result = deregister_all(
                    entries,
                    protocol,
                    operation_timeout,
                    deadline,
                    Some(waiter_dropped),
                    cancellation_timeout,
                )
                .await;
                let _ = completion.send(result);
                return;
            }
        }
    }

    let deadline = Instant::now() + cancellation_timeout;
    if let Err(error) = deregister_all(
        entries,
        protocol,
        operation_timeout,
        Some(deadline),
        None,
        cancellation_timeout,
    )
    .await
    {
        tracing::error!(
            ?error,
            "background service deregistration did not complete cleanly"
        );
    }
}

async fn deregister_all(
    entries: Vec<RegistrationEntry>,
    protocol: WireProtocol,
    operation_timeout: Duration,
    mut deadline: Option<Instant>,
    mut waiter_dropped: Option<oneshot::Receiver<()>>,
    cancellation_timeout: Duration,
) -> Result<(), RegistrationCleanupError> {
    let mut first_error = None;
    let mut timed_out = false;
    for (registry, resource) in entries.into_iter().rev() {
        let service = resource.selector().service_id().to_string();
        let now = Instant::now();
        let operation_deadline = now + operation_timeout;
        let effective_deadline = deadline
            .map(|deadline| deadline.min(operation_deadline))
            .unwrap_or(operation_deadline);
        if effective_deadline <= now {
            timed_out = true;
            tracing::error!(%service, "service deregistration skipped after shutdown deadline");
            continue;
        }
        let deregister = registry.deregister(resource, protocol);
        tokio::pin!(deregister);
        let mut operation_result = None;
        // Startup rollback has no total deadline until its caller is cancelled.
        let cancellation_started = if deadline.is_none() {
            if let Some(waiter_dropped) = waiter_dropped.as_mut() {
                tokio::select! {
                    biased;
                    _ = waiter_dropped => true,
                    result = tokio::time::timeout_at(operation_deadline, &mut deregister) => {
                        operation_result = Some(result);
                        false
                    }
                }
            } else {
                false
            }
        } else {
            false
        };
        if cancellation_started {
            waiter_dropped = None;
            let cancellation_deadline = Instant::now() + cancellation_timeout;
            deadline = Some(cancellation_deadline);
            operation_result = Some(
                tokio::time::timeout_at(
                    operation_deadline.min(cancellation_deadline),
                    &mut deregister,
                )
                .await,
            );
        } else if operation_result.is_none() {
            operation_result =
                Some(tokio::time::timeout_at(effective_deadline, &mut deregister).await);
        }
        match operation_result.expect("deregistration operation was polled") {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::error!(?error, %service, "service deregistration failed");
                if first_error.is_none() {
                    first_error = Some(RegistrationCleanupError::Deregister {
                        service,
                        source: error,
                    });
                }
            }
            Err(_) => {
                timed_out = true;
                tracing::error!(%service, "service deregistration timed out");
            }
        }
    }

    if timed_out {
        Err(RegistrationCleanupError::Timeout)
    } else if let Some(error) = first_error {
        Err(error)
    } else {
        Ok(())
    }
}

fn server_result(outcome: TcpServerOutcome<RegistrationCleanupError>) -> Result<(), FusenError> {
    let TcpServerOutcome {
        accept_error,
        shutdown,
    } = outcome;
    match shutdown {
        ShutdownCompletion::DeadlineExceeded { cleanup } => {
            if let Some(Err(cleanup_error)) = cleanup {
                tracing::error!(
                    ?cleanup_error,
                    "service deregistration also failed before the graceful shutdown deadline"
                );
            }
            if let Some(error) = accept_error {
                tracing::error!(
                    ?error,
                    "HTTP accept failed before the graceful shutdown deadline was exceeded"
                );
            }
            Err(FusenError::Timeout(
                "server graceful shutdown deadline exceeded".into(),
            ))
        }
        ShutdownCompletion::Completed(Err(RegistrationCleanupError::Timeout)) => {
            if let Some(error) = accept_error {
                tracing::error!(
                    ?error,
                    "HTTP accept failed before service deregistration timed out"
                );
            }
            Err(FusenError::Timeout(
                "service deregistration deadline exceeded".into(),
            ))
        }
        ShutdownCompletion::Completed(cleanup) => {
            if let Some(error) = accept_error {
                if let Err(cleanup_error) = cleanup {
                    tracing::error!(
                        ?cleanup_error,
                        "service deregistration also failed after the HTTP accept error"
                    );
                }
                Err(FusenError::internal("HTTP server failed", error))
            } else {
                cleanup
                    .map_err(|error| FusenError::internal("service deregistration failed", error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn shutdown_error_priority_is_stable() {
        assert!(matches!(
            server_result(TcpServerOutcome {
                accept_error: Some(accept_error()),
                shutdown: ShutdownCompletion::DeadlineExceeded {
                    cleanup: Some(Err(deregister_error())),
                },
            }),
            Err(FusenError::Timeout(_))
        ));

        assert!(matches!(
            server_result(TcpServerOutcome {
                accept_error: Some(accept_error()),
                shutdown: ShutdownCompletion::Completed(Err(RegistrationCleanupError::Timeout,)),
            }),
            Err(FusenError::Timeout(_))
        ));

        assert!(matches!(
            server_result(TcpServerOutcome {
                accept_error: Some(accept_error()),
                shutdown: ShutdownCompletion::Completed(Err(deregister_error())),
            }),
            Err(FusenError::Internal {
                message: "HTTP server failed",
                ..
            })
        ));

        assert!(matches!(
            server_result(TcpServerOutcome {
                accept_error: None,
                shutdown: ShutdownCompletion::Completed(Err(deregister_error())),
            }),
            Err(FusenError::Internal {
                message: "service deregistration failed",
                ..
            })
        ));

        assert!(
            server_result(TcpServerOutcome {
                accept_error: None,
                shutdown: ShutdownCompletion::Completed(Ok(())),
            })
            .is_ok()
        );
    }

    fn accept_error() -> io::Error {
        io::Error::other("accept failed")
    }

    fn deregister_error() -> RegistrationCleanupError {
        RegistrationCleanupError::Deregister {
            service: "test-service".into(),
            source: RegisterError::InvalidResource("deregister failed".into()),
        }
    }
}
