use crate::{
    error::FusenError,
    handler::{Handler, HandlerContext, HandlerInfo},
    protocol::{
        codec::FusenHttpCodec,
        http::server::{TcpServer, TcpServerConfig},
    },
    server::{
        path::PathCache,
        router::{Router, RouterContext},
        rpc::{RpcServerHandler, RpcService},
    },
};
use fusen_internal_common::{
    protocol::WireProtocol,
    resource::service::{MethodResource, ParameterResource, ServiceResource},
};
use fusen_register::Register;
use std::{collections::HashMap, future::Future, net::SocketAddr, sync::Arc, time::Duration};
use tokio::time::Instant;
use tokio::{net::TcpListener, sync::Semaphore};

#[allow(missing_docs)]
pub mod path;
#[allow(missing_docs)]
pub mod router;
#[allow(missing_docs)]
pub mod rpc;

#[derive(Clone, Debug)]
/// Typed server address, protocol, resource limits, and shutdown deadlines.
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

/// Transactional builder for RPC services, handlers, and registries.
pub struct FusenServerBuilder {
    config: ServerConfig,
    registers: Vec<Arc<dyn Register>>,
    handler_context: HandlerContext,
    service_handlers: Vec<HandlerInfo>,
    services: HashMap<String, Box<dyn RpcService>>,
}

impl FusenServerBuilder {
    /// Creates a server builder bound to the supplied socket address.
    pub fn new(bind_addr: SocketAddr) -> Self {
        Self {
            config: ServerConfig::new(bind_addr),
            registers: Vec::new(),
            handler_context: HandlerContext::default(),
            service_handlers: Vec::new(),
            services: HashMap::new(),
        }
    }

    /// Replaces all typed server settings.
    pub fn config(mut self, config: ServerConfig) -> Self {
        self.config = config;
        self
    }

    /// Adds a registry participating in startup and shutdown transactions.
    pub fn register(mut self, register: impl Register + 'static) -> Self {
        self.registers.push(Arc::new(register));
        self
    }

    /// Adds one uniquely identified middleware handler.
    pub fn handler(mut self, handler: Handler) -> Result<Self, FusenError> {
        self.handler_context.load_handler(handler)?;
        Ok(self)
    }

    /// Adds one generated RPC service and its ordered handlers.
    pub fn service(
        mut self,
        (service, handlers): (Box<dyn RpcService>, Option<Vec<&str>>),
    ) -> Result<Self, FusenError> {
        let info = service.get_service_info();
        info.validate()?;
        let tag = info.service_desc.get_tag().to_owned();
        if self.services.contains_key(&tag) {
            return Err(FusenError::InvalidRequest(format!(
                "duplicate service {tag}"
            )));
        }
        self.service_handlers.push(HandlerInfo {
            service_desc: info.service_desc.clone(),
            handlers: handlers
                .unwrap_or_default()
                .into_iter()
                .map(str::to_owned)
                .collect(),
        });
        self.services.insert(tag, service);
        Ok(self)
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

    /// Runs with a caller-provided shutdown future, primarily for embedding and tests.
    pub async fn run_with_shutdown<S>(mut self, shutdown: S) -> Result<(), FusenError>
    where
        S: Future<Output = ()> + Send,
    {
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
        let advertised = if self.registers.is_empty() {
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
        let mut method_infos = Vec::new();
        let mut resources = Vec::new();
        let mut service_infos = self
            .services
            .values()
            .map(|service| service.get_service_info())
            .collect::<Vec<_>>();
        service_infos.sort_by(|left, right| {
            left.service_desc
                .get_tag()
                .cmp(right.service_desc.get_tag())
        });
        for info in service_infos {
            info.validate()?;
            method_infos.extend(info.method_infos.iter().cloned().map(Arc::new));
            resources.push(Arc::new(ServiceResource {
                service_id: info.service_desc.service_id,
                group: info.service_desc.group,
                version: info.service_desc.version,
                methods: info
                    .method_infos
                    .into_iter()
                    .map(|method| MethodResource {
                        method_name: method.method_name,
                        path: method.path,
                        method: method.method.to_string(),
                        parameters: method
                            .parameters
                            .into_iter()
                            .map(|parameter| ParameterResource {
                                name: parameter.name,
                                source: parameter.source,
                            })
                            .collect(),
                    })
                    .collect(),
                addr: advertised.clone(),
                weight: None,
                metadata: Default::default(),
            }));
        }
        let path_cache = PathCache::build(method_infos)?;
        for controller in self.service_handlers {
            self.handler_context.load_controller(controller)?;
        }
        let listener = TcpListener::bind(self.config.bind_addr)
            .await
            .map_err(|error| FusenError::internal("failed to bind server socket", error))?;

        let mut registered = Vec::new();
        for register in &self.registers {
            for resource in &resources {
                registered.push((register.clone(), resource.clone()));
                match tokio::time::timeout(
                    self.config.registry_timeout,
                    register.register(resource.clone(), self.config.protocol),
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
        let router = Router {
            context: Arc::new(RouterContext {
                http_codec: FusenHttpCodec::new(self.config.max_request_body_bytes),
                path_cache,
                handler_context: self.handler_context,
                fusen_service_handler: RpcServerHandler::new(self.services),
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
}

fn validate_advertised_url(value: &str) -> Result<String, FusenError> {
    let url =
        url::Url::parse(value).map_err(|error| FusenError::InvalidRequest(error.to_string()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.port_or_known_default().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(FusenError::InvalidRequest(
            "advertised_base_url must be an absolute HTTP(S) URL without query or fragment".into(),
        ));
    }
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

async fn rollback(
    entries: &[(Arc<dyn Register>, Arc<ServiceResource>)],
    protocol: WireProtocol,
    operation_timeout: Duration,
    deadline: Option<Instant>,
) {
    for (register, resource) in entries.iter().rev() {
        let remaining = deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(operation_timeout);
        let timeout = operation_timeout.min(remaining);
        if timeout.is_zero() {
            tracing::error!(service = %resource.service_id, "service deregistration skipped after shutdown deadline");
            break;
        }
        match tokio::time::timeout(timeout, register.deregister(resource.clone(), protocol)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::error!(?error, service = %resource.service_id, "service deregistration failed");
            }
            Err(_) => {
                tracing::error!(service = %resource.service_id, "service deregistration timed out");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        filter::{FusenFilter, ProceedingJoinPoint},
        protocol::fusen::{
            context::FusenContext,
            service::{MethodInfo, ServiceDesc, ServiceInfo},
        },
    };
    use fusen_internal_common::BoxFuture;
    use fusen_register::{ServiceSubscription, error::RegisterError};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::{Notify, oneshot},
    };

    #[derive(Clone)]
    struct MockRegister {
        registered: Arc<AtomicUsize>,
        deregistered: Arc<AtomicUsize>,
        fail_on: Option<usize>,
        hang_on: Option<usize>,
        hang_deregister: bool,
    }

    impl Register for MockRegister {
        fn register(
            &self,
            _resource: Arc<ServiceResource>,
            _protocol: WireProtocol,
        ) -> BoxFuture<Result<(), RegisterError>> {
            let count = self.registered.fetch_add(1, Ordering::SeqCst) + 1;
            let fail = self.fail_on == Some(count);
            let hang = self.hang_on == Some(count);
            Box::pin(async move {
                if hang {
                    std::future::pending::<()>().await;
                }
                if fail {
                    Err(RegisterError::provider(std::io::Error::other(
                        "register failed",
                    )))
                } else {
                    Ok(())
                }
            })
        }
        fn deregister(
            &self,
            _resource: Arc<ServiceResource>,
            _protocol: WireProtocol,
        ) -> BoxFuture<Result<(), RegisterError>> {
            let counter = self.deregistered.clone();
            let hang = self.hang_deregister;
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                if hang {
                    std::future::pending::<()>().await;
                }
                Ok(())
            })
        }
        fn subscribe(
            &self,
            _resource: ServiceResource,
            _protocol: WireProtocol,
        ) -> BoxFuture<Result<ServiceSubscription, RegisterError>> {
            Box::pin(async { Ok(ServiceSubscription::local(Vec::new())) })
        }
    }

    struct DummyService(&'static str);
    impl FusenFilter for DummyService {
        fn call<'a>(
            &'a self,
            join_point: ProceedingJoinPoint,
        ) -> fusen_internal_common::BoxFutureV2<'a, Result<FusenContext, FusenError>> {
            Box::pin(async move { Ok(join_point.context) })
        }
    }

    struct SlowService {
        started: Arc<Notify>,
        delay: Duration,
    }

    impl FusenFilter for SlowService {
        fn call<'a>(
            &'a self,
            join_point: ProceedingJoinPoint,
        ) -> fusen_internal_common::BoxFutureV2<'a, Result<FusenContext, FusenError>> {
            Box::pin(async move {
                let mut context = join_point.context;
                self.started.notify_one();
                tokio::time::sleep(self.delay).await;
                let response = crate::protocol::fusen::response::FusenResponse {
                    body: Some(serde_json::json!("ok")),
                    ..Default::default()
                };
                context.response = Some(response);
                Ok(context)
            })
        }
    }

    impl RpcService for SlowService {
        fn get_service_info(&self) -> ServiceInfo {
            let service = ServiceDesc::new("slow", None, None);
            ServiceInfo::new(
                service.clone(),
                vec![MethodInfo::new(
                    service,
                    "get".into(),
                    http::Method::GET,
                    "/slow".into(),
                    Vec::new(),
                )],
            )
        }
    }
    impl RpcService for DummyService {
        fn get_service_info(&self) -> ServiceInfo {
            let service = ServiceDesc::new(self.0, None, None);
            ServiceInfo::new(
                service.clone(),
                vec![MethodInfo::new(
                    service,
                    "get".into(),
                    http::Method::GET,
                    format!("/{}", self.0),
                    Vec::new(),
                )],
            )
        }
    }

    fn mock(fail_on: Option<usize>) -> MockRegister {
        MockRegister {
            registered: Arc::new(AtomicUsize::new(0)),
            deregistered: Arc::new(AtomicUsize::new(0)),
            fail_on,
            hang_on: None,
            hang_deregister: false,
        }
    }

    #[tokio::test]
    async fn bind_failure_happens_before_registration() {
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = occupied.local_addr().unwrap();
        let register = mock(None);
        let registered = register.registered.clone();
        let mut config = ServerConfig::new(addr);
        config.advertised_base_url = Some(format!("http://{addr}"));
        let server = FusenServerBuilder::new(addr)
            .config(config)
            .register(register)
            .service((Box::new(DummyService("one")), None))
            .unwrap();
        assert!(
            server
                .run_with_shutdown(std::future::pending())
                .await
                .is_err()
        );
        assert_eq!(registered.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn partial_registration_is_rolled_back() {
        let register = mock(Some(2));
        let deregistered = register.deregistered.clone();
        let addr = "127.0.0.1:0".parse().unwrap();
        let mut config = ServerConfig::new(addr);
        config.advertised_base_url = Some("http://127.0.0.1:8080".into());
        let server = FusenServerBuilder::new(addr)
            .config(config)
            .register(register)
            .service((Box::new(DummyService("one")), None))
            .unwrap()
            .service((Box::new(DummyService("two")), None))
            .unwrap();
        assert!(
            server
                .run_with_shutdown(std::future::pending())
                .await
                .is_err()
        );
        assert_eq!(deregistered.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn timed_out_registration_compensates_the_uncertain_resource() {
        let mut register = mock(None);
        register.hang_on = Some(1);
        let deregistered = register.deregistered.clone();
        let addr = "127.0.0.1:0".parse().unwrap();
        let mut config = ServerConfig::new(addr);
        config.advertised_base_url = Some("http://127.0.0.1:8080".into());
        config.registry_timeout = Duration::from_millis(20);
        let server = FusenServerBuilder::new(addr)
            .config(config)
            .register(register)
            .service((Box::new(DummyService("one")), None))
            .unwrap();
        assert!(matches!(
            server.run_with_shutdown(std::future::pending()).await,
            Err(FusenError::Timeout(_))
        ));
        assert_eq!(deregistered.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn hanging_deregistration_cannot_extend_shutdown_deadline() {
        let mut register = mock(None);
        register.hang_deregister = true;
        let addr = available_addr();
        let mut config = ServerConfig::new(addr);
        config.advertised_base_url = Some(format!("http://{addr}"));
        config.registry_timeout = Duration::from_secs(1);
        config.graceful_shutdown_timeout = Duration::from_millis(30);
        let server = FusenServerBuilder::new(addr)
            .config(config)
            .register(register)
            .service((Box::new(DummyService("one")), None))
            .unwrap();
        let start = tokio::time::Instant::now();
        server.run_with_shutdown(async {}).await.unwrap();
        assert!(start.elapsed() < Duration::from_millis(200));
    }

    async fn connect(addr: SocketAddr) -> tokio::net::TcpStream {
        for _ in 0..100 {
            if let Ok(stream) = tokio::net::TcpStream::connect(addr).await {
                return stream;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("server did not start listening");
    }

    fn available_addr() -> SocketAddr {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    }

    #[tokio::test]
    async fn graceful_shutdown_completes_in_flight_request() {
        let addr = available_addr();
        let started = Arc::new(Notify::new());
        let notified = started.notified();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = FusenServerBuilder::new(addr)
            .service((
                Box::new(SlowService {
                    started: started.clone(),
                    delay: Duration::from_millis(50),
                }),
                None,
            ))
            .unwrap();
        let task = tokio::spawn(server.run_with_shutdown(async {
            let _ = shutdown_rx.await;
        }));
        let mut stream = connect(addr).await;
        stream
            .write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        notified.await;
        shutdown_tx.send(()).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        task.await.unwrap().unwrap();
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("\"ok\""));
    }

    #[tokio::test]
    async fn graceful_shutdown_aborts_after_deadline() {
        let addr = available_addr();
        let started = Arc::new(Notify::new());
        let notified = started.notified();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut config = ServerConfig::new(addr);
        config.graceful_shutdown_timeout = Duration::from_millis(20);
        let server = FusenServerBuilder::new(addr)
            .config(config)
            .service((
                Box::new(SlowService {
                    started: started.clone(),
                    delay: Duration::from_secs(60),
                }),
                None,
            ))
            .unwrap();
        let task = tokio::spawn(server.run_with_shutdown(async {
            let _ = shutdown_rx.await;
        }));
        let mut stream = connect(addr).await;
        stream
            .write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        notified.await;
        let start = tokio::time::Instant::now();
        shutdown_tx.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn connection_limit_holds_permit_for_socket_lifetime() {
        let addr = available_addr();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut config = ServerConfig::new(addr);
        config.max_connections = 1;
        let server = FusenServerBuilder::new(addr)
            .config(config)
            .service((
                Box::new(SlowService {
                    started: Arc::new(Notify::new()),
                    delay: Duration::ZERO,
                }),
                None,
            ))
            .unwrap();
        let task = tokio::spawn(server.run_with_shutdown(async {
            let _ = shutdown_rx.await;
        }));
        let first = connect(addr).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut second = tokio::net::TcpStream::connect(addr).await.unwrap();
        second
            .write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut probe = [0_u8; 1];
        assert!(
            tokio::time::timeout(Duration::from_millis(20), second.read(&mut probe))
                .await
                .is_err()
        );
        drop(first);
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), second.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200"));
        shutdown_tx.send(()).unwrap();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn http1_slow_headers_are_closed_after_timeout() {
        let addr = available_addr();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut config = ServerConfig::new(addr);
        config.http1_header_read_timeout = Duration::from_millis(30);
        let server = FusenServerBuilder::new(addr)
            .config(config)
            .service((Box::new(DummyService("one")), None))
            .unwrap();
        let task = tokio::spawn(server.run_with_shutdown(async {
            let _ = shutdown_rx.await;
        }));
        let mut stream = connect(addr).await;
        stream.write_all(b"GET /one HTTP/1.1\r\n").await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), stream.read_to_end(&mut response))
            .await
            .expect("slow HTTP/1 headers were not closed")
            .unwrap();
        assert!(response.is_empty());
        shutdown_tx.send(()).unwrap();
        task.await.unwrap().unwrap();
    }
}
