use crate::{
    error::FusenError,
    handler::{Handler, HandlerContext, HandlerInfo},
    protocol::{codec::FusenHttpCodec, http::server::TcpServer},
    server::{
        path::PathCache,
        router::{Router, RouterContext},
        rpc::{RpcServerHandler, RpcService},
    },
};
use fusen_internal_common::{
    protocol::WireProtocol,
    resource::service::{MethodResource, ServiceResource},
};
use fusen_register::Register;
use std::{collections::HashMap, future::Future, net::SocketAddr, sync::Arc, time::Duration};
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
    /// End-to-end server deadline for one request.
    pub request_timeout: Duration,
    /// Maximum time allowed for active connections to drain.
    pub graceful_shutdown_timeout: Duration,
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
            request_timeout: Duration::from_secs(30),
            graceful_shutdown_timeout: Duration::from_secs(30),
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
        if self.config.max_request_body_bytes == 0 || self.config.max_concurrent_requests == 0 {
            return Err(FusenError::InvalidRequest(
                "server limits must be greater than zero".into(),
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
        let mut method_infos = Vec::new();
        let mut resources = Vec::new();
        for service in self.services.values() {
            let info = service.get_service_info();
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
                if let Err(error) = register
                    .register(resource.clone(), self.config.protocol)
                    .await
                {
                    rollback(&registered, self.config.protocol).await;
                    return Err(FusenError::internal("service registration failed", error));
                }
                registered.push((register.clone(), resource.clone()));
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
        let on_shutdown = async move {
            rollback(&registered, protocol).await;
        };
        TcpServer::run(
            listener,
            router,
            shutdown,
            on_shutdown,
            self.config.graceful_shutdown_timeout,
        )
        .await
        .map_err(|error| FusenError::internal("HTTP server failed", error))
    }
}

async fn rollback(entries: &[(Arc<dyn Register>, Arc<ServiceResource>)], protocol: WireProtocol) {
    for (register, resource) in entries.iter().rev() {
        if let Err(error) = register.deregister(resource.clone(), protocol).await {
            tracing::error!(?error, service = %resource.service_id, "service deregistration failed");
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
    use fusen_register::{directory::Directory, error::RegisterError};
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
    }

    impl Register for MockRegister {
        fn register(
            &self,
            _resource: Arc<ServiceResource>,
            _protocol: WireProtocol,
        ) -> BoxFuture<Result<(), RegisterError>> {
            let count = self.registered.fetch_add(1, Ordering::SeqCst) + 1;
            let fail = self.fail_on == Some(count);
            Box::pin(async move {
                if fail {
                    Err(RegisterError::Error(Box::new(std::io::Error::other(
                        "register failed",
                    ))))
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
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
        fn subscribe(
            &self,
            _resource: ServiceResource,
            _protocol: WireProtocol,
        ) -> BoxFuture<Result<Directory, RegisterError>> {
            Box::pin(async { Ok(Directory::default()) })
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
                    "GET".into(),
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
                    "GET".into(),
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
        assert_eq!(deregistered.load(Ordering::SeqCst), 1);
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
}
