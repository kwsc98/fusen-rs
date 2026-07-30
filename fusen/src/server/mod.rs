mod config;
mod http;
mod routes;
mod transport;

use crate::{
    Middleware, ServerError, ServerErrorKind,
    middleware::erase_middleware,
    runtime::metrics::SafeMetrics,
    server::{
        http::{HttpApp, HttpAppConfig},
        routes::{Route, RouteTable},
        transport::{AcceptOutcome, DrainCommand, TransportConfig},
    },
    service::{IntoServerService, PreparedService},
};
use fusen_contract::{
    InstanceId, ProtocolSet, ServiceDescriptor, ServiceEndpoint, ServiceRegistration,
    ServiceWeight, WireProtocol,
};
use fusen_observability::{
    MetricEvent, MetricOutcome, MetricsRecorder, RegistryOperationEvent, ShutdownFinishedEvent,
};
use fusen_register::{RegistrationHandle, RegistrationRequest, Registry};
use futures_util::{StreamExt, future::join_all, stream};
use std::{
    collections::HashSet,
    net::SocketAddr,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Instant as StdInstant,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

pub use config::{
    HttpServerConfig, HttpServerConfigBuilder, ServerConfig, ServerConfigBuilder,
    ServerRegistryConfig, ServerRegistryConfigBuilder, ServerRequestConfig,
    ServerRequestConfigBuilder,
};

pub(crate) const NOT_READY: u8 = 0;
pub(crate) const READY: u8 = 1;
pub(crate) const DRAINING: u8 = 2;
pub(crate) const STOPPED: u8 = 3;

/// Observable server lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServerState {
    /// Static validation completed but the socket has not been bound.
    Validated,
    /// The listener is accepting, while requests still receive `not_ready`.
    AcceptingNotReady,
    /// Registration handles are activating.
    Registering,
    /// The server accepts RPC work.
    Ready,
    /// Admission and the listener are closed while existing work drains.
    Draining,
    /// All lifecycle work reached a terminal result.
    Stopped,
}

impl ServerState {
    const fn as_u8(self) -> u8 {
        match self {
            Self::Validated => 0,
            Self::AcceptingNotReady => 1,
            Self::Registering => 2,
            Self::Ready => 3,
            Self::Draining => 4,
            Self::Stopped => 5,
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Validated,
            1 => Self::AcceptingNotReady,
            2 => Self::Registering,
            3 => Self::Ready,
            4 => Self::Draining,
            _ => Self::Stopped,
        }
    }
}

pub(crate) struct Readiness(AtomicU8);

impl Readiness {
    fn new() -> Self {
        Self(AtomicU8::new(NOT_READY))
    }

    pub(crate) fn load(&self) -> u8 {
        self.0.load(Ordering::Acquire)
    }

    fn store(&self, value: u8) {
        self.0.store(value, Ordering::Release);
    }
}

struct NamedRegistry {
    name: Arc<str>,
    registry: Arc<dyn Registry>,
}

/// Validated plaintext HTTP/1.1 and h2c server that has not yet bound its listener.
pub struct Server {
    address: SocketAddr,
    advertised_endpoint: Option<ServiceEndpoint>,
    config: ServerConfig,
    registries: Vec<NamedRegistry>,
    descriptors: Vec<&'static ServiceDescriptor>,
    routes: Arc<RouteTable>,
    metrics: SafeMetrics,
}

/// Builder for a clean-slate [`Server`].
pub struct ServerBuilder {
    address: Result<SocketAddr, String>,
    advertised_endpoint: Option<Result<ServiceEndpoint, String>>,
    config: ServerConfig,
    registries: Vec<NamedRegistry>,
    head_middleware: Vec<Arc<dyn Middleware>>,
    middleware: Vec<Arc<dyn Middleware>>,
    services: Vec<PreparedService>,
    metrics: Option<Arc<dyn MetricsRecorder>>,
}

impl Server {
    /// Creates a server builder for an IPv4 or IPv6 socket address.
    pub fn builder(address: impl AsRef<str>) -> ServerBuilder {
        ServerBuilder {
            address: address
                .as_ref()
                .parse::<SocketAddr>()
                .map_err(|error| error.to_string()),
            advertised_endpoint: None,
            config: ServerConfig::default(),
            registries: Vec::new(),
            head_middleware: Vec::new(),
            middleware: Vec::new(),
            services: Vec::new(),
            metrics: None,
        }
    }

    /// Binds, starts accepting in not-ready mode, activates registrations, and returns at Ready.
    pub async fn start(self) -> Result<RunningServer, ServerError> {
        let listener = TcpListener::bind(self.address).await.map_err(|error| {
            ServerError::with_source(
                ServerErrorKind::Bind,
                "failed to bind listening socket",
                error,
            )
        })?;
        let local_addr = listener.local_addr().map_err(|error| {
            ServerError::with_source(
                ServerErrorKind::Bind,
                "failed to read bound listening address",
                error,
            )
        })?;
        let advertised = match self.advertised_endpoint {
            Some(endpoint) => endpoint,
            None => format!("http://{local_addr}")
                .parse::<ServiceEndpoint>()
                .map_err(|error| {
                    ServerError::with_source(
                        ServerErrorKind::Validation,
                        "failed to construct advertised plaintext endpoint",
                        error,
                    )
                })?,
        };
        let instance_id =
            InstanceId::new(uuid::Uuid::new_v4().simple().to_string()).map_err(|error| {
                ServerError::with_source(
                    ServerErrorKind::Validation,
                    "failed to create provider instance identity",
                    error,
                )
            })?;
        let registrations = build_registration_plan(
            &self.registries,
            &self.descriptors,
            &self.config,
            instance_id,
            advertised,
        )?;

        let readiness = Arc::new(Readiness::new());
        let state = Arc::new(AtomicU8::new(ServerState::Validated.as_u8()));
        let shutdown = CancellationToken::new();
        let (completion_sender, completion) = watch::channel(None);
        let (startup_sender, startup) = oneshot::channel();
        let request = self.config.request();
        let http_config = self.config.http();
        let app = HttpApp::new(
            self.routes,
            readiness.clone(),
            HttpAppConfig {
                protocols: self.config.protocols(),
                request_timeout: request.timeout(),
                max_uri_bytes: http_config.max_uri_bytes(),
                max_query_pairs: http_config.max_query_pairs(),
                max_headers: http_config.max_headers(),
                max_header_bytes: http_config.max_header_bytes(),
                max_request_body: request.max_request_body_bytes(),
                max_response_body: request.max_response_body_bytes(),
                max_concurrent_requests: request.max_concurrent_requests(),
                queue_capacity: request.queue_capacity(),
                queue_max_wait: request.queue_max_wait(),
                request_byte_budget: request.max_inflight_request_body_bytes(),
                response_byte_budget: request.max_inflight_response_body_bytes(),
            },
            self.metrics.clone(),
        );
        let inner = Arc::new(ServerHandleInner {
            local_addr,
            state: state.clone(),
            shutdown: shutdown.clone(),
            completion,
        });
        tokio::spawn(coordinate(Coordinator {
            listener: Some(listener),
            app,
            readiness,
            state,
            shutdown: shutdown.clone(),
            startup: Some(startup_sender),
            completion: completion_sender,
            registrations,
            config: self.config,
            metrics: self.metrics,
        }));

        let mut guard = StartupGuard {
            shutdown,
            armed: true,
        };
        match startup.await {
            Ok(Ok(())) => {
                guard.armed = false;
                Ok(RunningServer { inner })
            }
            Ok(Err(error)) => Err(error),
            Err(error) => Err(ServerError::with_source(
                ServerErrorKind::Startup,
                "server startup coordinator ended without a result",
                error,
            )),
        }
    }

    /// Starts the server, listens for the platform shutdown signal, and drains it.
    pub async fn serve(self) -> Result<(), ServerError> {
        let running = self.start().await?;
        let handle = running.handle();
        tokio::select! {
            result = handle.wait() => result,
            () = default_shutdown_signal() => running.shutdown().await,
        }
    }
}

impl ServerBuilder {
    /// Replaces the immutable server configuration.
    pub fn config(mut self, config: ServerConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the externally reachable HTTP or HTTPS endpoint published to registries.
    ///
    /// HTTPS describes an external TLS terminator and does not enable TLS on the local listener.
    pub fn advertised_endpoint(mut self, endpoint: impl AsRef<str>) -> Self {
        self.advertised_endpoint = Some(
            endpoint
                .as_ref()
                .parse::<ServiceEndpoint>()
                .map_err(|error| error.to_string()),
        );
        self
    }

    /// Appends a named registry in deterministic insertion order.
    pub fn registry<R>(mut self, name: impl AsRef<str>, registry: R) -> Self
    where
        R: Registry + 'static,
    {
        self.registries.push(NamedRegistry {
            name: Arc::from(name.as_ref()),
            registry: Arc::new(registry),
        });
        self
    }

    /// Appends global server middleware in execution order.
    pub fn middleware(mut self, middleware: impl Middleware) -> Self {
        self.middleware.push(erase_middleware(middleware));
        self
    }

    /// Appends global middleware that runs before an accepted request body is polled.
    pub fn head_middleware(mut self, middleware: impl Middleware) -> Self {
        self.head_middleware.push(erase_middleware(middleware));
        self
    }

    /// Installs the synchronous, non-blocking metrics recorder.
    pub fn metrics(mut self, recorder: impl MetricsRecorder) -> Self {
        self.metrics = Some(Arc::new(recorder));
        self
    }

    /// Adds one macro-generated interface server.
    pub fn interface(mut self, interface: impl IntoServerService) -> Self {
        self.services.push(interface.into_server_service());
        self
    }

    /// Validates routes, resources, and extension identities without performing network I/O.
    pub fn build(self) -> Result<Server, ServerError> {
        let address = self.address.map_err(|error| {
            ServerError::message(
                ServerErrorKind::Validation,
                format!("invalid server socket address: {error}"),
            )
        })?;
        let advertised_endpoint = self
            .advertised_endpoint
            .transpose()
            .map_err(|error| ServerError::message(ServerErrorKind::Validation, error))?;
        self.config.validate().map_err(|error| {
            ServerError::with_source(
                ServerErrorKind::Validation,
                format!("invalid server configuration at {}", error.field_path()),
                error,
            )
        })?;
        validate_registry_names(&self.registries)?;
        if self.services.is_empty() {
            return Err(ServerError::message(
                ServerErrorKind::Validation,
                "server must contain at least one service",
            ));
        }
        let mut descriptors = HashSet::new();
        let mut routes = Vec::new();
        let mut descriptor_list = Vec::new();
        for prepared in self.services {
            let descriptor = prepared.descriptor().map_err(|reason| {
                ServerError::message(
                    ServerErrorKind::Validation,
                    format!("invalid interface schema: {reason}"),
                )
            })?;
            if !descriptors.insert(descriptor.identity()) {
                return Err(ServerError::message(
                    ServerErrorKind::Validation,
                    format!("duplicate service identity {}", descriptor.identity()),
                ));
            }
            if !self
                .config
                .protocols()
                .is_subset_of(descriptor.supported_protocols())
            {
                return Err(ServerError::message(
                    ServerErrorKind::Validation,
                    format!(
                        "service {} does not implement every enabled wire protocol",
                        descriptor.identity()
                    ),
                ));
            }
            let mut middleware = Vec::with_capacity(
                self.middleware
                    .len()
                    .saturating_add(prepared.middleware.len()),
            );
            middleware.extend(self.middleware.iter().cloned());
            middleware.extend(prepared.middleware.iter().cloned());
            let middleware: Arc<[Arc<dyn Middleware>]> = Arc::from(middleware);
            let mut head_middleware = Vec::with_capacity(
                self.head_middleware
                    .len()
                    .saturating_add(prepared.head_middleware.len()),
            );
            head_middleware.extend(self.head_middleware.iter().cloned());
            head_middleware.extend(prepared.head_middleware.iter().cloned());
            let head_middleware: Arc<[Arc<dyn Middleware>]> = Arc::from(head_middleware);
            for protocol in self.config.protocols().iter() {
                for method in descriptor.methods() {
                    routes.push(Route {
                        protocol,
                        service: descriptor,
                        method,
                        dispatch: prepared.dispatch.clone(),
                        head_middleware: head_middleware.clone(),
                        middleware: middleware.clone(),
                    });
                }
            }
            descriptor_list.push(descriptor);
        }
        descriptor_list.sort_by(|left, right| left.identity().cmp(right.identity()));
        let routes = RouteTable::build(routes)
            .map_err(|error| ServerError::message(ServerErrorKind::Validation, error))?;
        Ok(Server {
            address,
            advertised_endpoint,
            config: self.config,
            registries: self.registries,
            descriptors: descriptor_list,
            routes: Arc::new(routes),
            metrics: SafeMetrics::new(self.metrics),
        })
    }
}

/// A bound server that reached Ready.
pub struct RunningServer {
    inner: Arc<ServerHandleInner>,
}

impl RunningServer {
    /// Returns the actual once-bound listening address.
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr
    }

    /// Returns the latest lifecycle state.
    pub fn state(&self) -> ServerState {
        ServerState::from_u8(self.inner.state.load(Ordering::Acquire))
    }

    /// Returns a cloneable shutdown and wait handle.
    pub fn handle(&self) -> ServerHandle {
        ServerHandle {
            inner: self.inner.clone(),
        }
    }

    /// Waits for a fatal accept failure or external shutdown.
    pub async fn wait(self) -> Result<(), ServerError> {
        self.handle().wait().await
    }

    /// Requests idempotent shutdown and waits for its shared terminal result.
    pub async fn shutdown(self) -> Result<(), ServerError> {
        self.handle().shutdown().await
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.inner.shutdown.cancel();
    }
}

/// Cloneable control handle for one running server.
#[derive(Clone)]
pub struct ServerHandle {
    inner: Arc<ServerHandleInner>,
}

impl ServerHandle {
    /// Returns the latest lifecycle state.
    pub fn state(&self) -> ServerState {
        ServerState::from_u8(self.inner.state.load(Ordering::Acquire))
    }

    /// Requests shutdown once and waits for the shared terminal result.
    pub async fn shutdown(&self) -> Result<(), ServerError> {
        self.inner.shutdown.cancel();
        self.wait().await
    }

    /// Waits for the shared terminal result without initiating shutdown.
    pub async fn wait(&self) -> Result<(), ServerError> {
        let mut completion = self.inner.completion.clone();
        loop {
            if let Some(result) = completion.borrow().clone() {
                return result;
            }
            completion.changed().await.map_err(|error| {
                ServerError::with_source(
                    ServerErrorKind::Shutdown,
                    "server completion channel closed without a terminal result",
                    error,
                )
            })?;
        }
    }
}

struct ServerHandleInner {
    local_addr: SocketAddr,
    state: Arc<AtomicU8>,
    shutdown: CancellationToken,
    completion: watch::Receiver<Option<Result<(), ServerError>>>,
}

struct StartupGuard {
    shutdown: CancellationToken,
    armed: bool,
}

impl Drop for StartupGuard {
    fn drop(&mut self) {
        if self.armed {
            self.shutdown.cancel();
        }
    }
}

struct PlannedRegistration {
    name: Arc<str>,
    registry: Arc<dyn Registry>,
    registration: Arc<ServiceRegistration>,
    protocol: WireProtocol,
}

#[derive(Clone)]
struct TrackedRegistration {
    name: Arc<str>,
    handle: RegistrationHandle,
}

struct Coordinator {
    listener: Option<TcpListener>,
    app: HttpApp,
    readiness: Arc<Readiness>,
    state: Arc<AtomicU8>,
    shutdown: CancellationToken,
    startup: Option<oneshot::Sender<Result<(), ServerError>>>,
    completion: watch::Sender<Option<Result<(), ServerError>>>,
    registrations: Vec<PlannedRegistration>,
    config: ServerConfig,
    metrics: SafeMetrics,
}

async fn coordinate(mut coordinator: Coordinator) {
    let (drain_sender, drain_receiver) = mpsc::unbounded_channel();
    let (fatal_sender, mut fatal_receiver) = mpsc::unbounded_channel();
    let (accept_sender, mut accept_receiver) = oneshot::channel();
    let force_cancel = CancellationToken::new();
    let http = coordinator.config.http();
    let transport_config = TransportConfig {
        max_connections: http.max_connections(),
        max_headers: http.max_headers(),
        max_request_head_bytes: http.max_uri_bytes().saturating_add(http.max_header_bytes()),
        http1_header_read_timeout: http.http1_header_read_timeout(),
        http2_max_concurrent_streams: http.http2_max_concurrent_streams(),
        http2_keep_alive_interval: http.http2_keep_alive_interval(),
        http2_keep_alive_timeout: http.http2_keep_alive_timeout(),
    };
    let listener = coordinator
        .listener
        .take()
        .expect("server coordinator owns its listener until transport starts");
    tokio::spawn(transport::run(
        listener,
        coordinator.app.clone(),
        transport_config,
        drain_receiver,
        force_cancel.clone(),
        fatal_sender,
        accept_sender,
    ));
    set_state(&coordinator.state, ServerState::AcceptingNotReady);
    set_state(&coordinator.state, ServerState::Registering);

    let mut tracked = Vec::with_capacity(coordinator.registrations.len());
    let prepare_result = prepare_registrations(&coordinator.registrations, &mut tracked);
    let startup_deadline = Instant::now() + coordinator.config.registry().startup_timeout();
    let activation = async {
        prepare_result?;
        activate_registrations(
            tracked.clone(),
            coordinator.config.registry().operation_timeout(),
            coordinator.config.registry().max_concurrent_operations(),
            coordinator.metrics.clone(),
        )
        .await
    };
    let startup_cancelled = coordinator.shutdown.clone().cancelled_owned();
    let mut startup_fatal = None;
    let startup_result = tokio::select! {
        biased;
        () = startup_cancelled => Err(ServerError::message(
            ServerErrorKind::Startup,
            "server startup was cancelled",
        )),
        error = fatal_receiver.recv() => {
            startup_fatal = error;
            Err(ServerError::message(
                ServerErrorKind::Accept,
                "HTTP accept supervisor failed before the server reached Ready",
            ))
        },
        result = tokio::time::timeout_at(startup_deadline, activation) => match result {
            Ok(result) => result,
            Err(_) => Err(ServerError::message(
                ServerErrorKind::Startup,
                "server did not reach Ready before the startup deadline",
            )),
        }
    };

    if let Err(startup_error) = startup_result {
        let result = match drain_runtime(
            &coordinator,
            tracked.clone(),
            &drain_sender,
            &mut accept_receiver,
            &force_cancel,
            startup_fatal,
        )
        .await
        {
            Ok(()) => Err(startup_error),
            Err(shutdown_error)
                if matches!(
                    shutdown_error.kind(),
                    ServerErrorKind::Timeout | ServerErrorKind::Accept
                ) =>
            {
                Err(shutdown_error)
            }
            Err(rollback_error) => {
                tracing::error!(
                    ?rollback_error,
                    ?startup_error,
                    "registration rollback failed after server startup failed"
                );
                Err(startup_error)
            }
        };
        if let Some(startup) = coordinator.startup.take() {
            let _ = startup.send(result.clone());
        }
        finish(&coordinator, result);
        return;
    }

    coordinator.readiness.store(READY);
    set_state(&coordinator.state, ServerState::Ready);
    if let Some(startup) = coordinator.startup.take() {
        let _ = startup.send(Ok(()));
    }

    let shutdown_cancelled = coordinator.shutdown.clone().cancelled_owned();
    let fatal = tokio::select! {
        biased;
        () = shutdown_cancelled => None,
        error = fatal_receiver.recv() => error,
    };
    let result = drain_runtime(
        &coordinator,
        tracked,
        &drain_sender,
        &mut accept_receiver,
        &force_cancel,
        fatal,
    )
    .await;
    finish(&coordinator, result);
}

fn finish(coordinator: &Coordinator, result: Result<(), ServerError>) {
    coordinator.readiness.store(STOPPED);
    set_state(&coordinator.state, ServerState::Stopped);
    coordinator.completion.send_replace(Some(result));
}

async fn drain_runtime(
    coordinator: &Coordinator,
    tracked: Vec<TrackedRegistration>,
    drain: &mpsc::UnboundedSender<DrainCommand>,
    accept: &mut oneshot::Receiver<AcceptOutcome>,
    force_cancel: &CancellationToken,
    fatal: Option<std::io::Error>,
) -> Result<(), ServerError> {
    coordinator.readiness.store(DRAINING);
    coordinator.app.begin_draining();
    let deadline = Instant::now() + coordinator.config.graceful_shutdown_timeout();
    let (listener_closed_sender, listener_closed) = oneshot::channel();
    let _ = drain.send(DrainCommand {
        deadline,
        listener_closed: listener_closed_sender,
    });
    let started = StdInstant::now();
    let work = async {
        let listener_error =
            publish_draining_after_listener_closed(listener_closed, &coordinator.state).await;
        let close = close_registrations(
            tracked,
            deadline,
            coordinator.config.registry().operation_timeout(),
            coordinator.config.registry().max_concurrent_operations(),
            coordinator.metrics.clone(),
        );
        let (registry, (), accept) = tokio::join!(close, coordinator.app.drained(), accept);
        (registry, accept, listener_error)
    };
    let result = match tokio::time::timeout_at(deadline, work).await {
        Err(_) => {
            force_cancel.cancel();
            Err(ServerError::message(
                ServerErrorKind::Timeout,
                "server graceful shutdown deadline elapsed",
            ))
        }
        Ok((registry, accept, listener_error)) => resolve_shutdown_result(
            registry,
            accept.map_err(|error| {
                ServerError::with_source(
                    ServerErrorKind::Accept,
                    "HTTP accept supervisor ended without an outcome",
                    error,
                )
            }),
            fatal,
            listener_error,
        ),
    };
    coordinator
        .metrics
        .record(&MetricEvent::ShutdownFinished(ShutdownFinishedEvent::new(
            "server",
            match &result {
                Ok(()) => MetricOutcome::Success,
                Err(error) if error.kind() == ServerErrorKind::Timeout => MetricOutcome::Timeout,
                Err(_) => MetricOutcome::Error,
            },
            started.elapsed(),
        )));
    result
}

async fn publish_draining_after_listener_closed(
    listener_closed: oneshot::Receiver<()>,
    state: &AtomicU8,
) -> Option<ServerError> {
    let error = listener_closed.await.err().map(|error| {
        ServerError::with_source(
            ServerErrorKind::Accept,
            "HTTP accept supervisor ended before confirming listener closure",
            error,
        )
    });
    if error.is_none() {
        set_state(state, ServerState::Draining);
    }
    error
}

fn prepare_registrations(
    plan: &[PlannedRegistration],
    tracked: &mut Vec<TrackedRegistration>,
) -> Result<(), ServerError> {
    for item in plan {
        let prepared = catch_unwind(AssertUnwindSafe(|| {
            item.registry.prepare_registration(RegistrationRequest::new(
                item.registration.clone(),
                item.protocol,
            ))
        }));
        let handle = match prepared {
            Ok(Ok(handle)) => handle,
            Ok(Err(error)) => {
                return Err(ServerError::with_source(
                    ServerErrorKind::Registry,
                    format!("registry {} rejected registration preparation", item.name),
                    error,
                ));
            }
            Err(_) => {
                return Err(ServerError::message(
                    ServerErrorKind::Registry,
                    format!(
                        "registry {} panicked while preparing registration",
                        item.name
                    ),
                ));
            }
        };
        tracked.push(TrackedRegistration {
            name: item.name.clone(),
            handle,
        });
    }
    Ok(())
}

async fn activate_registrations(
    tracked: Vec<TrackedRegistration>,
    operation_timeout: std::time::Duration,
    concurrency: usize,
    metrics: SafeMetrics,
) -> Result<(), ServerError> {
    let operations = stream::iter(tracked.into_iter().map(|tracked| {
        let handle = tracked.handle;
        let name = tracked.name;
        let metrics = metrics.clone();
        async move {
            let started = StdInstant::now();
            let result = tokio::time::timeout(operation_timeout, handle.activate()).await;
            metrics.record(&MetricEvent::RegistryOperation(
                RegistryOperationEvent::new(
                    &name,
                    "activate_registration",
                    match &result {
                        Ok(Ok(())) => MetricOutcome::Success,
                        Err(_) => MetricOutcome::Timeout,
                        Ok(Err(_)) => MetricOutcome::Error,
                    },
                    started.elapsed(),
                ),
            ));
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => Err(ServerError::with_source(
                    ServerErrorKind::Registry,
                    format!("registry {name} failed to activate registration"),
                    error,
                )),
                Err(_) => Err(ServerError::message(
                    ServerErrorKind::Registry,
                    format!("registry {name} registration activation timed out"),
                )),
            }
        }
    }))
    .buffer_unordered(concurrency);
    tokio::pin!(operations);
    while let Some(result) = operations.next().await {
        result?;
    }
    Ok(())
}

struct CloseOutcome {
    timed_out: bool,
    first_error: Option<fusen_register::error::RegistryError>,
}

fn resolve_shutdown_result(
    registry: CloseOutcome,
    accept: Result<AcceptOutcome, ServerError>,
    fatal: Option<std::io::Error>,
    listener_error: Option<ServerError>,
) -> Result<(), ServerError> {
    let (accept_deadline_exceeded, outcome_fatal, completion_error) = match accept {
        Ok(accept) => (accept.deadline_exceeded, accept.fatal_error, None),
        Err(error) => (false, None, Some(error)),
    };
    let accept_error = fatal.or(outcome_fatal);
    if registry.timed_out || accept_deadline_exceeded {
        if let Some(error) = registry.first_error {
            tracing::error!(?error, "registry error was overridden by shutdown timeout");
        }
        if let Some(error) = accept_error.as_ref() {
            tracing::error!(?error, "accept error was overridden by shutdown timeout");
        }
        if let Some(error) = completion_error.as_ref() {
            tracing::error!(
                ?error,
                "accept completion error was overridden by shutdown timeout"
            );
        }
        if let Some(error) = listener_error.as_ref() {
            tracing::error!(
                ?error,
                "listener closure error was overridden by shutdown timeout"
            );
        }
        return Err(ServerError::message(
            ServerErrorKind::Timeout,
            "server graceful shutdown deadline elapsed",
        ));
    }
    if let Some(error) = accept_error {
        if let Some(completion_error) = completion_error {
            tracing::error!(
                ?completion_error,
                "accept completion error was overridden by the fatal accept failure"
            );
        }
        if let Some(listener_error) = listener_error {
            tracing::error!(
                ?listener_error,
                "listener closure error was overridden by the fatal accept failure"
            );
        }
        if let Some(registry_error) = registry.first_error {
            tracing::error!(
                ?registry_error,
                "registry error was overridden by accept failure"
            );
        }
        return Err(ServerError::with_source(
            ServerErrorKind::Accept,
            "HTTP server reached a fatal accept failure",
            error,
        ));
    }
    if let Some(error) = listener_error {
        if let Some(completion_error) = completion_error {
            tracing::error!(
                ?completion_error,
                "accept completion error was overridden by listener closure failure"
            );
        }
        if let Some(registry_error) = registry.first_error {
            tracing::error!(
                ?registry_error,
                "registry error was overridden by listener closure failure"
            );
        }
        return Err(error);
    }
    if let Some(error) = completion_error {
        if let Some(registry_error) = registry.first_error {
            tracing::error!(
                ?registry_error,
                "registry error was overridden by accept completion failure"
            );
        }
        return Err(error);
    }
    if let Some(error) = registry.first_error {
        Err(ServerError::with_source(
            ServerErrorKind::Registry,
            "one or more service deregistrations failed",
            error,
        ))
    } else {
        Ok(())
    }
}

async fn close_registrations(
    tracked: Vec<TrackedRegistration>,
    deadline: Instant,
    operation_timeout: std::time::Duration,
    concurrency: usize,
    metrics: SafeMetrics,
) -> CloseOutcome {
    let reversed = tracked.into_iter().rev().collect::<Vec<_>>();
    let mut outcome = CloseOutcome {
        timed_out: false,
        first_error: None,
    };
    for batch in reversed.chunks(concurrency) {
        if Instant::now() >= deadline {
            outcome.timed_out = true;
            break;
        }
        let futures = batch.iter().cloned().map(|tracked| {
            let handle = tracked.handle;
            let name = tracked.name;
            let metrics = metrics.clone();
            async move {
                let started = StdInstant::now();
                let operation_deadline = deadline.min(Instant::now() + operation_timeout);
                let result = tokio::time::timeout_at(operation_deadline, handle.close()).await;
                metrics.record(&MetricEvent::RegistryOperation(
                    RegistryOperationEvent::new(
                        &name,
                        "close_registration",
                        match &result {
                            Ok(Ok(())) => MetricOutcome::Success,
                            Err(_) => MetricOutcome::Timeout,
                            Ok(Err(_)) => MetricOutcome::Error,
                        },
                        started.elapsed(),
                    ),
                ));
                result
            }
        });
        for result in join_all(futures).await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::error!(?error, "service deregistration failed");
                    if outcome.first_error.is_none() {
                        outcome.first_error = Some(error);
                    }
                }
                Err(_) => outcome.timed_out = true,
            }
        }
    }
    outcome
}

fn build_registration_plan(
    registries: &[NamedRegistry],
    descriptors: &[&'static ServiceDescriptor],
    config: &ServerConfig,
    instance_id: InstanceId,
    endpoint: ServiceEndpoint,
) -> Result<Vec<PlannedRegistration>, ServerError> {
    let mut plan = Vec::new();
    for registry in registries {
        for protocol in config.protocols().iter() {
            for descriptor in descriptors {
                let registration = ServiceRegistration::new(
                    instance_id.clone(),
                    descriptor,
                    endpoint.clone(),
                    ProtocolSet::from_protocol(protocol),
                    ServiceWeight::default(),
                )
                .map_err(|error| {
                    ServerError::with_source(
                        ServerErrorKind::Validation,
                        "failed to create service registration",
                        error,
                    )
                })?;
                plan.push(PlannedRegistration {
                    name: registry.name.clone(),
                    registry: registry.registry.clone(),
                    registration: Arc::new(registration),
                    protocol,
                });
            }
        }
    }
    Ok(plan)
}

fn validate_registry_names(registries: &[NamedRegistry]) -> Result<(), ServerError> {
    let mut seen = HashSet::new();
    for registry in registries {
        let name = registry.name.as_ref();
        let valid = !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if !valid || !seen.insert(name) {
            return Err(ServerError::message(
                ServerErrorKind::Validation,
                format!("invalid or duplicate registry name {name:?}"),
            ));
        }
    }
    Ok(())
}

fn set_state(state: &AtomicU8, value: ServerState) {
    state.store(value.as_u8(), Ordering::Release);
}

async fn default_shutdown_signal() {
    #[cfg(unix)]
    {
        let Some(signals) = install_unix_shutdown_signals() else {
            return;
        };
        wait_for_unix_shutdown_signal(signals).await;
    }
    #[cfg(not(unix))]
    {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(?error, "Ctrl-C listener failed; triggering shutdown");
        }
    }
}

#[cfg(unix)]
struct UnixShutdownSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
fn install_unix_shutdown_signals() -> Option<UnixShutdownSignals> {
    use tokio::signal::unix::{SignalKind, signal};

    let interrupt = match signal(SignalKind::interrupt()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(
                ?error,
                "failed to install SIGINT listener; triggering shutdown"
            );
            return None;
        }
    };
    let terminate = match signal(SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(error) => {
            tracing::error!(
                ?error,
                "failed to install SIGTERM listener; triggering shutdown"
            );
            return None;
        }
    };
    Some(UnixShutdownSignals {
        interrupt,
        terminate,
    })
}

#[cfg(unix)]
async fn wait_for_unix_shutdown_signal(mut signals: UnixShutdownSignals) {
    tokio::select! {
        value = signals.interrupt.recv() => {
            if value.is_none() {
                tracing::error!("SIGINT listener closed; triggering shutdown");
            }
        }
        value = signals.terminate.recv() => {
            if value.is_none() {
                tracing::error!("SIGTERM listener closed; triggering shutdown");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusen_register::error::{RegistryError, RegistryErrorKind, RegistryOperation};

    #[cfg(unix)]
    const SIGNAL_CHILD_ENV: &str = "FUSEN_SIGNAL_SMOKE_CHILD";
    #[cfg(unix)]
    const SIGNAL_READY: &str = "FUSEN_SIGNAL_LISTENERS_READY";

    #[test]
    fn shutdown_result_prioritizes_timeout_over_accept_and_registry_errors() {
        let result = resolve_shutdown_result(
            CloseOutcome {
                timed_out: true,
                first_error: Some(registry_failure()),
            },
            Err(ServerError::message(
                ServerErrorKind::Accept,
                "accept completion missing",
            )),
            Some(std::io::Error::other("fatal accept")),
            Some(ServerError::message(
                ServerErrorKind::Accept,
                "listener closure missing",
            )),
        );
        assert_eq!(result.unwrap_err().kind(), ServerErrorKind::Timeout);
    }

    #[test]
    fn shutdown_result_prioritizes_accept_over_registry_error() {
        let result = resolve_shutdown_result(
            CloseOutcome {
                timed_out: false,
                first_error: Some(registry_failure()),
            },
            Ok(AcceptOutcome {
                fatal_error: Some(std::io::Error::other("fatal accept")),
                deadline_exceeded: false,
            }),
            None,
            None,
        );
        assert_eq!(result.unwrap_err().kind(), ServerErrorKind::Accept);
    }

    #[test]
    fn shutdown_result_prioritizes_listener_closure_over_completion_and_registry_errors() {
        let result = resolve_shutdown_result(
            CloseOutcome {
                timed_out: false,
                first_error: Some(registry_failure()),
            },
            Err(ServerError::message(
                ServerErrorKind::Accept,
                "accept completion missing",
            )),
            None,
            Some(ServerError::message(
                ServerErrorKind::Accept,
                "listener closure missing",
            )),
        );
        let error = result.unwrap_err();
        assert_eq!(error.kind(), ServerErrorKind::Accept);
        assert_eq!(error.message_ref(), "listener closure missing");
    }

    #[test]
    fn shutdown_result_returns_registry_error_after_clean_transport_drain() {
        let result = resolve_shutdown_result(
            CloseOutcome {
                timed_out: false,
                first_error: Some(registry_failure()),
            },
            Ok(AcceptOutcome {
                fatal_error: None,
                deadline_exceeded: false,
            }),
            None,
            None,
        );
        assert_eq!(result.unwrap_err().kind(), ServerErrorKind::Registry);
    }

    #[tokio::test]
    async fn draining_state_waits_for_listener_closure_acknowledgement() {
        let state = AtomicU8::new(ServerState::Ready.as_u8());
        let (listener_closed, acknowledgement) = oneshot::channel();
        let publish = publish_draining_after_listener_closed(acknowledgement, &state);
        tokio::pin!(publish);

        tokio::select! {
            biased;
            result = &mut publish => panic!("state advanced before listener ack: {result:?}"),
            () = tokio::task::yield_now() => {}
        }
        assert_eq!(
            ServerState::from_u8(state.load(Ordering::Acquire)),
            ServerState::Ready
        );

        listener_closed.send(()).unwrap();
        assert!(publish.await.is_none());
        assert_eq!(
            ServerState::from_u8(state.load(Ordering::Acquire)),
            ServerState::Draining
        );
    }

    #[cfg(unix)]
    #[test]
    fn sigint_and_sigterm_are_received_in_isolated_subprocesses() {
        use std::{
            io::{BufRead, BufReader},
            process::{Command, Stdio},
            sync::mpsc,
            thread,
            time::{Duration, Instant},
        };

        for signal in ["-INT", "-TERM"] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "server::tests::unix_signal_child", "--nocapture"])
                .env(SIGNAL_CHILD_ENV, "1")
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let stdout = child.stdout.take().unwrap();
            let (ready_sender, ready) = mpsc::sync_channel(1);
            thread::spawn(move || {
                let mut ready_sent = false;
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if !ready_sent && line.contains(SIGNAL_READY) {
                        let _ = ready_sender.send(());
                        ready_sent = true;
                    }
                }
            });
            if ready.recv_timeout(Duration::from_secs(5)).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                panic!("signal smoke child did not install its listeners");
            }
            let delivered = Command::new("kill")
                .args([signal, &child.id().to_string()])
                .status()
                .unwrap();
            assert!(delivered.success(), "failed to deliver {signal}");

            let deadline = Instant::now() + Duration::from_secs(5);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("signal smoke child did not stop after {signal}");
                }
                thread::sleep(Duration::from_millis(10));
            };
            assert!(status.success(), "signal smoke child failed for {signal}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_signal_child() {
        use std::io::Write;

        if std::env::var_os(SIGNAL_CHILD_ENV).is_none() {
            return;
        }
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let signals = install_unix_shutdown_signals().expect("install signal listeners");
                println!("{SIGNAL_READY}");
                std::io::stdout().flush().unwrap();
                wait_for_unix_shutdown_signal(signals).await;
            });
    }

    fn registry_failure() -> RegistryError {
        RegistryError::message(
            RegistryOperation::CloseRegistration,
            RegistryErrorKind::Unavailable,
            "registry close failed",
        )
    }
}
