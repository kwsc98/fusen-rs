use crate::{
    ClientConfig, ClientRuntime,
    contract::{ServiceRegistration, ServiceSelector, StaticBoxFuture, WireProtocol},
    error::FusenError,
    filter::{Middleware, Next, RpcResult},
    fusen_service, fusen_trait,
    invocation::{
        InvocationFinish, InvocationObserver, InvocationOutcome, InvocationPhase, InvocationSide,
        InvocationStart,
    },
    protocol::fusen::context::RpcContext,
    server::{Server, ServerConfig},
};
use fusen_register::{Register, ServiceSubscription, error::RegisterError};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{Semaphore, mpsc, oneshot},
};

#[fusen_trait(id = "protocol-e2e")]
#[crate::asset(path = "/rpc", method = POST)]
trait ProtocolService {
    #[crate::asset(path = "/items/{id}")]
    async fn echo(&self, id: String, values: Vec<i32>) -> Vec<i32>;

    async fn ping(&self) -> String;

    async fn fail(&self) -> String;

    async fn slow(&self) -> String;

    #[crate::asset(path = "/lookup/{id}", method = GET)]
    async fn lookup(&self, id: String, limit: Option<u32>) -> String;
}

struct ProtocolServiceImpl;

#[fusen_service]
impl ProtocolService for ProtocolServiceImpl {
    async fn lookup(&self, id: String, limit: Option<u32>) -> Result<String, FusenError> {
        Ok(format!("{id}:{}", limit.unwrap_or_default()))
    }

    async fn slow(&self) -> Result<String, FusenError> {
        tokio::time::sleep(Duration::from_millis(100)).await;
        Ok("slow".into())
    }

    async fn ping(&self) -> Result<String, FusenError> {
        Ok("pong".into())
    }

    async fn fail(&self) -> Result<String, FusenError> {
        Err(FusenError::InvalidRequest("expected failure".into()))
    }

    async fn echo(&self, id: String, values: Vec<i32>) -> Result<Vec<i32>, FusenError> {
        assert_eq!(id, "a/b space");
        Ok(values)
    }
}

#[fusen_trait(id = "shutdown-e2e")]
trait ShutdownService {
    async fn wait_for_release(&self) -> String;
}

struct ShutdownServiceImpl {
    started: Mutex<Option<oneshot::Sender<()>>>,
    release: Arc<Semaphore>,
}

#[fusen_service]
impl ShutdownService for ShutdownServiceImpl {
    async fn wait_for_release(&self) -> Result<String, FusenError> {
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(());
        }
        let permit = self
            .release
            .acquire()
            .await
            .expect("shutdown test release semaphore closed");
        permit.forget();
        Ok("drained".into())
    }
}

#[derive(Clone)]
enum RegisterAction {
    Complete,
    Fail(&'static str),
    Wait(Arc<Semaphore>),
}

#[derive(Debug, PartialEq, Eq)]
enum RegistryEvent {
    RegisterStarted(&'static str),
    RegisterFinished(&'static str),
    RegisterCancelled(&'static str),
    DeregisterStarted(&'static str),
    DeregisterFinished(&'static str),
    RegistryDropped(&'static str),
}

struct RegisterCallGuard {
    name: &'static str,
    events: mpsc::UnboundedSender<RegistryEvent>,
    completed: bool,
}

impl Drop for RegisterCallGuard {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self
                .events
                .send(RegistryEvent::RegisterCancelled(self.name));
        }
    }
}

struct ControlledRegister {
    name: &'static str,
    register_action: RegisterAction,
    deregister_action: RegisterAction,
    events: mpsc::UnboundedSender<RegistryEvent>,
    deregister_count: Arc<AtomicUsize>,
}

impl ControlledRegister {
    fn new(
        name: &'static str,
        register_action: RegisterAction,
        deregister_action: RegisterAction,
        events: mpsc::UnboundedSender<RegistryEvent>,
    ) -> (Self, Arc<AtomicUsize>) {
        let deregister_count = Arc::new(AtomicUsize::new(0));
        (
            Self {
                name,
                register_action,
                deregister_action,
                events,
                deregister_count: deregister_count.clone(),
            },
            deregister_count,
        )
    }
}

impl Drop for ControlledRegister {
    fn drop(&mut self) {
        let _ = self.events.send(RegistryEvent::RegistryDropped(self.name));
    }
}

impl Register for ControlledRegister {
    fn register(
        &self,
        _registration: Arc<ServiceRegistration>,
        _protocol: WireProtocol,
    ) -> StaticBoxFuture<Result<(), RegisterError>> {
        let name = self.name;
        let action = self.register_action.clone();
        let events = self.events.clone();
        Box::pin(async move {
            let _ = events.send(RegistryEvent::RegisterStarted(name));
            let mut guard = RegisterCallGuard {
                name,
                events: events.clone(),
                completed: false,
            };
            let result = run_register_action(action).await;
            guard.completed = true;
            let _ = events.send(RegistryEvent::RegisterFinished(name));
            result
        })
    }

    fn deregister(
        &self,
        _registration: Arc<ServiceRegistration>,
        _protocol: WireProtocol,
    ) -> StaticBoxFuture<Result<(), RegisterError>> {
        let name = self.name;
        let action = self.deregister_action.clone();
        let events = self.events.clone();
        let deregister_count = self.deregister_count.clone();
        Box::pin(async move {
            deregister_count.fetch_add(1, Ordering::SeqCst);
            let _ = events.send(RegistryEvent::DeregisterStarted(name));
            let result = run_register_action(action).await;
            let _ = events.send(RegistryEvent::DeregisterFinished(name));
            result
        })
    }

    fn subscribe(
        &self,
        _selector: ServiceSelector,
        _protocol: WireProtocol,
    ) -> StaticBoxFuture<Result<ServiceSubscription, RegisterError>> {
        Box::pin(async { Ok(ServiceSubscription::local(Vec::new())) })
    }
}

async fn run_register_action(action: RegisterAction) -> Result<(), RegisterError> {
    match action {
        RegisterAction::Complete => Ok(()),
        RegisterAction::Fail(message) => Err(RegisterError::InvalidResource(message.into())),
        RegisterAction::Wait(gate) => {
            let permit = gate
                .acquire_owned()
                .await
                .map_err(|_| RegisterError::InvalidResource("test gate closed".into()))?;
            permit.forget();
            Ok(())
        }
    }
}

#[tokio::test]
async fn spring_http1_generated_client_covers_routing_body_errors_and_deadline() {
    let address = available_address();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address).service(ProtocolServiceImpl);
    let task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));
    wait_until_listening(address).await;

    let runtime = ClientRuntime::builder().build().unwrap();
    let client = ProtocolServiceClient::builder(&runtime)
        .direct(format!("http://{address}"))
        .protocol(WireProtocol::SpringCloud)
        .connect()
        .await
        .unwrap();
    assert_eq!(
        client
            .echo("a/b space".into(), vec![4, 5, 6])
            .await
            .unwrap(),
        vec![4, 5, 6]
    );
    assert_eq!(
        client.lookup("item/one".into(), Some(7)).await.unwrap(),
        "item/one:7"
    );
    let error = client.fail().await.unwrap_err();
    let FusenError::Remote(problem) = error else {
        panic!("expected a remote Problem Details error");
    };
    assert_eq!(problem.code, "invalid_request");
    assert!(uuid::Uuid::parse_str(&problem.request_id).is_ok());
    runtime.shutdown().await.unwrap();

    let config = ClientConfig {
        request_timeout: Duration::from_millis(20),
        ..ClientConfig::default()
    };
    let timeout_runtime = ClientRuntime::builder().config(config).build().unwrap();
    let timeout_client = ProtocolServiceClient::builder(&timeout_runtime)
        .direct(format!("http://{address}"))
        .protocol(WireProtocol::SpringCloud)
        .connect()
        .await
        .unwrap();
    assert!(matches!(
        timeout_client.slow().await,
        Err(FusenError::Timeout(_))
    ));
    timeout_runtime.shutdown().await.unwrap();

    shutdown_sender.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("server shutdown exceeded its test deadline")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn fusen_http2_round_trip_preserves_single_array_body() {
    let address = available_address();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address).service(ProtocolServiceImpl);
    let task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));
    wait_until_listening(address).await;

    let runtime = ClientRuntime::builder().build().unwrap();
    let client = ProtocolServiceClient::builder(&runtime)
        .direct(format!("http://{address}"))
        .connect()
        .await
        .unwrap();
    let response = client
        .echo("a/b space".into(), vec![1, 2, 3])
        .await
        .unwrap();
    assert_eq!(response, vec![1, 2, 3]);
    assert_eq!(client.ping().await.unwrap(), "pong");
    assert!(matches!(
        client.fail().await,
        Err(FusenError::Remote(problem)) if problem.code == "invalid_request"
    ));
    assert_eq!(
        ProtocolServiceClient::service_descriptor().methods()[0]
            .id()
            .get(),
        0
    );
    assert_eq!(
        ProtocolServiceClient::service_descriptor().methods()[1]
            .id()
            .get(),
        1
    );
    assert!(std::ptr::eq(
        ProtocolServiceClient::service_descriptor(),
        <ProtocolServiceImpl as crate::server::rpc::RpcServiceInfo>::service_descriptor(
            &ProtocolServiceImpl,
        ),
    ));
    runtime.shutdown().await.unwrap();

    shutdown_sender.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("server shutdown exceeded its test deadline")
        .unwrap()
        .unwrap();
}

#[derive(Clone, Default)]
struct LifecycleObserver(Arc<Mutex<Vec<(InvocationSide, InvocationOutcome, InvocationPhase)>>>);

impl InvocationObserver for LifecycleObserver {
    fn on_start(&self, _event: &InvocationStart<'_>) {}

    fn on_finish(&self, event: &InvocationFinish<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((event.side, event.outcome, event.phase));
    }
}

#[tokio::test]
async fn server_timeout_is_observed_once_and_returned_as_remote_problem() {
    let address = available_address();
    let observer = LifecycleObserver::default();
    let mut config = ServerConfig::new(address);
    config.request_timeout = Duration::from_millis(20);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address)
        .config(config)
        .observer(observer.clone())
        .service(ProtocolServiceImpl);
    let task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));
    wait_until_listening(address).await;

    let runtime = ClientRuntime::builder().build().unwrap();
    let client = ProtocolServiceClient::builder(&runtime)
        .direct(format!("http://{address}"))
        .connect()
        .await
        .unwrap();
    assert!(matches!(
        client.slow().await,
        Err(FusenError::Remote(problem)) if problem.code == "timeout"
    ));
    assert_eq!(
        *observer.0.lock().unwrap(),
        [(
            InvocationSide::Server,
            InvocationOutcome::Timeout,
            InvocationPhase::Service,
        )]
    );
    runtime.shutdown().await.unwrap();
    shutdown_sender.send(()).unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn route_failure_is_observed_once() {
    let address = available_address();
    let observer = LifecycleObserver::default();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address)
        .observer(observer.clone())
        .service(ProtocolServiceImpl);
    let task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));
    wait_until_listening(address).await;

    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"GET /missing HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    assert!(
        String::from_utf8(response)
            .unwrap()
            .starts_with("HTTP/1.1 404")
    );
    assert_eq!(
        *observer.0.lock().unwrap(),
        [(
            InvocationSide::Server,
            InvocationOutcome::Error,
            InvocationPhase::Route,
        )]
    );
    shutdown_sender.send(()).unwrap();
    task.await.unwrap().unwrap();
}

struct RecordingMiddleware {
    name: &'static str,
    events: Arc<Mutex<Vec<String>>>,
}

impl Middleware for RecordingMiddleware {
    async fn handle<'a>(&'a self, context: RpcContext, next: Next<'a>) -> RpcResult {
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:in", self.name));
        let result = next.run(context).await;
        self.events
            .lock()
            .unwrap()
            .push(format!("{}:out", self.name));
        result
    }
}

#[tokio::test]
async fn global_and_service_middleware_wrap_in_documented_order() {
    let address = available_address();
    let events = Arc::new(Mutex::new(Vec::new()));
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address)
        .middleware(RecordingMiddleware {
            name: "server-global",
            events: events.clone(),
        })
        .service(
            ProtocolServiceServer::new(ProtocolServiceImpl).middleware(RecordingMiddleware {
                name: "server-local",
                events: events.clone(),
            }),
        );
    let task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));
    wait_until_listening(address).await;

    let runtime = ClientRuntime::builder()
        .middleware(RecordingMiddleware {
            name: "client-global",
            events: events.clone(),
        })
        .build()
        .unwrap();
    let client = ProtocolServiceClient::builder(&runtime)
        .direct(format!("http://{address}"))
        .middleware(RecordingMiddleware {
            name: "client-local",
            events: events.clone(),
        })
        .connect()
        .await
        .unwrap();
    assert_eq!(client.ping().await.unwrap(), "pong");
    assert_eq!(
        *events.lock().unwrap(),
        [
            "client-global:in",
            "client-local:in",
            "server-global:in",
            "server-local:in",
            "server-local:out",
            "server-global:out",
            "client-local:out",
            "client-global:out",
        ]
    );
    runtime.shutdown().await.unwrap();
    shutdown_sender.send(()).unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn registration_failure_compensates_uncertain_entry_in_reverse_order() {
    let address = available_address();
    let (events_sender, mut events) = mpsc::unbounded_channel();
    let (first, first_deregister_count) = ControlledRegister::new(
        "first",
        RegisterAction::Complete,
        RegisterAction::Complete,
        events_sender.clone(),
    );
    let (second, second_deregister_count) = ControlledRegister::new(
        "second",
        RegisterAction::Fail("registration failed"),
        RegisterAction::Complete,
        events_sender,
    );
    let server = Server::bind(address)
        .config(registered_server_config(address, Duration::from_secs(1)))
        .registry(first)
        .registry(second)
        .service(ProtocolServiceImpl);

    let error = tokio::time::timeout(
        Duration::from_secs(2),
        server.run_with_shutdown(std::future::pending()),
    )
    .await
    .expect("registration rollback exceeded its test deadline")
    .unwrap_err();
    assert!(matches!(
        error,
        FusenError::Internal {
            message: "service registration failed",
            ..
        }
    ));
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("first")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("first")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("second")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("second")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterStarted("second")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterFinished("second")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterStarted("first")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterFinished("first")
    );
    assert_eq!(first_deregister_count.load(Ordering::SeqCst), 1);
    assert_eq!(second_deregister_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn abort_during_pending_registration_deregisters_once() {
    let address = available_address();
    let (events_sender, mut events) = mpsc::unbounded_channel();
    let (registry, deregister_count) = ControlledRegister::new(
        "pending",
        RegisterAction::Wait(Arc::new(Semaphore::new(0))),
        RegisterAction::Complete,
        events_sender,
    );
    let server = Server::bind(address)
        .config(registered_server_config(address, Duration::from_secs(1)))
        .registry(registry)
        .service(ProtocolServiceImpl);
    let task = tokio::spawn(server.run_with_shutdown(std::future::pending()));

    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("pending")
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterCancelled("pending")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterStarted("pending")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterFinished("pending")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegistryDropped("pending")
    );
    assert_eq!(deregister_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn abort_after_server_starts_deregisters_in_reverse_order_once() {
    let address = available_address();
    let (events_sender, mut events) = mpsc::unbounded_channel();
    let (first, first_deregister_count) = ControlledRegister::new(
        "first-running",
        RegisterAction::Complete,
        RegisterAction::Complete,
        events_sender.clone(),
    );
    let (second, second_deregister_count) = ControlledRegister::new(
        "second-running",
        RegisterAction::Complete,
        RegisterAction::Complete,
        events_sender,
    );
    let server = Server::bind(address)
        .config(registered_server_config(address, Duration::from_secs(1)))
        .registry(first)
        .registry(second)
        .service(ProtocolServiceImpl);
    let server_task = tokio::spawn(server.run_with_shutdown(std::future::pending()));

    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("first-running")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("first-running")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("second-running")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("second-running")
    );

    let runtime = ClientRuntime::builder().build().unwrap();
    let client = ProtocolServiceClient::builder(&runtime)
        .direct(format!("http://{address}"))
        .connect()
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), client.ping())
            .await
            .expect("server did not accept a request before its test deadline")
            .unwrap(),
        "pong"
    );

    server_task.abort();
    assert!(server_task.await.unwrap_err().is_cancelled());
    let mut cleanup_events = Vec::new();
    let mut dropped_registries = 0;
    while dropped_registries < 2 {
        match recv_registry_event(&mut events).await {
            RegistryEvent::RegistryDropped(_) => dropped_registries += 1,
            event
            @ (RegistryEvent::DeregisterStarted(_) | RegistryEvent::DeregisterFinished(_)) => {
                cleanup_events.push(event)
            }
            event => panic!("unexpected registry event during background cleanup: {event:?}"),
        }
    }
    assert_eq!(
        cleanup_events,
        [
            RegistryEvent::DeregisterStarted("second-running"),
            RegistryEvent::DeregisterFinished("second-running"),
            RegistryEvent::DeregisterStarted("first-running"),
            RegistryEvent::DeregisterFinished("first-running"),
        ]
    );
    assert_eq!(first_deregister_count.load(Ordering::SeqCst), 1);
    assert_eq!(second_deregister_count.load(Ordering::SeqCst), 1);
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn abort_during_startup_rollback_applies_the_graceful_deadline() {
    let address = available_address();
    let (events_sender, mut events) = mpsc::unbounded_channel();
    let (registry, deregister_count) = ControlledRegister::new(
        "startup-rollback",
        RegisterAction::Fail("registration failed"),
        RegisterAction::Wait(Arc::new(Semaphore::new(0))),
        events_sender,
    );
    let server = Server::bind(address)
        .config(registered_server_config(
            address,
            Duration::from_millis(200),
        ))
        .registry(registry)
        .service(ProtocolServiceImpl);
    let task = tokio::spawn(server.run_with_shutdown(std::future::pending()));

    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("startup-rollback")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("startup-rollback")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterStarted("startup-rollback")
    );

    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegistryDropped("startup-rollback")
    );
    assert_eq!(deregister_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shutdown_returns_internal_when_deregistration_fails() {
    let address = available_address();
    let (events_sender, mut events) = mpsc::unbounded_channel();
    let (remaining, remaining_deregister_count) = ControlledRegister::new(
        "remaining",
        RegisterAction::Complete,
        RegisterAction::Complete,
        events_sender.clone(),
    );
    let (failing, failing_deregister_count) = ControlledRegister::new(
        "failing",
        RegisterAction::Complete,
        RegisterAction::Fail("deregistration failed"),
        events_sender,
    );
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address)
        .config(registered_server_config(address, Duration::from_secs(1)))
        .registry(remaining)
        .registry(failing)
        .service(ProtocolServiceImpl);
    let task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));

    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("remaining")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("remaining")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("failing")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("failing")
    );
    shutdown_sender.send(()).unwrap();
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterStarted("failing")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterFinished("failing")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterStarted("remaining")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterFinished("remaining")
    );
    let error = await_server(task).await.unwrap_err();
    assert!(matches!(
        error,
        FusenError::Internal {
            message: "service deregistration failed",
            ..
        }
    ));
    assert_eq!(remaining_deregister_count.load(Ordering::SeqCst), 1);
    assert_eq!(failing_deregister_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shutdown_returns_timeout_when_deregistration_exceeds_shared_deadline() {
    let address = available_address();
    let (events_sender, mut events) = mpsc::unbounded_channel();
    let (registry, deregister_count) = ControlledRegister::new(
        "blocked",
        RegisterAction::Complete,
        RegisterAction::Wait(Arc::new(Semaphore::new(0))),
        events_sender,
    );
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address)
        .config(registered_server_config(
            address,
            Duration::from_millis(200),
        ))
        .registry(registry)
        .service(ProtocolServiceImpl);
    let task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));

    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("blocked")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("blocked")
    );
    shutdown_sender.send(()).unwrap();
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterStarted("blocked")
    );
    assert!(matches!(
        await_server(task).await,
        Err(FusenError::Timeout(_))
    ));
    assert_eq!(deregister_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn spring_http1_active_request_drains_during_shutdown() {
    active_request_drains_during_shutdown(WireProtocol::SpringCloud).await;
}

#[tokio::test]
async fn fusen_http2_active_request_drains_during_shutdown() {
    active_request_drains_during_shutdown(WireProtocol::Fusen).await;
}

async fn active_request_drains_during_shutdown(protocol: WireProtocol) {
    let address = available_address();
    let (request_started_sender, request_started_receiver) = oneshot::channel();
    let request_release = Arc::new(Semaphore::new(0));
    let deregister_release = Arc::new(Semaphore::new(0));
    let (events_sender, mut events) = mpsc::unbounded_channel();
    let (registry, deregister_count) = ControlledRegister::new(
        "drain",
        RegisterAction::Complete,
        RegisterAction::Wait(deregister_release.clone()),
        events_sender,
    );
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address)
        .config(registered_server_config(address, Duration::from_secs(5)))
        .registry(registry)
        .service(ShutdownServiceImpl {
            started: Mutex::new(Some(request_started_sender)),
            release: request_release.clone(),
        });
    let server_task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));

    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("drain")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("drain")
    );
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = ShutdownServiceClient::builder(&runtime)
        .direct(format!("http://{address}"))
        .protocol(protocol)
        .connect()
        .await
        .unwrap();
    let request_task = tokio::spawn(async move { client.wait_for_release().await });
    tokio::time::timeout(Duration::from_secs(2), request_started_receiver)
        .await
        .expect("request did not start before its test deadline")
        .expect("request start signal was dropped");

    shutdown_sender.send(()).unwrap();
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterStarted("drain")
    );
    let reconnect = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(address))
        .await
        .expect("connection attempt did not finish before its test deadline");
    assert!(
        reconnect.is_err(),
        "server listener accepted a connection after shutdown began"
    );

    request_release.add_permits(1);
    let response = tokio::time::timeout(Duration::from_secs(2), request_task)
        .await
        .expect("active request did not drain before its test deadline")
        .unwrap()
        .unwrap();
    assert_eq!(response, "drained");
    deregister_release.add_permits(1);
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterFinished("drain")
    );
    runtime.shutdown().await.unwrap();
    await_server(server_task).await.unwrap();
    assert_eq!(deregister_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn abort_after_explicit_cleanup_starts_deregisters_once() {
    let address = available_address();
    let (events_sender, mut events) = mpsc::unbounded_channel();
    let deregister_gate = Arc::new(Semaphore::new(0));
    let (registry, deregister_count) = ControlledRegister::new(
        "explicit-cleanup",
        RegisterAction::Complete,
        RegisterAction::Wait(deregister_gate.clone()),
        events_sender,
    );
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address)
        .config(registered_server_config(address, Duration::from_secs(1)))
        .registry(registry)
        .service(ProtocolServiceImpl);
    let server_task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));

    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterStarted("explicit-cleanup")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegisterFinished("explicit-cleanup")
    );
    shutdown_sender.send(()).unwrap();
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterStarted("explicit-cleanup")
    );

    server_task.abort();
    assert!(server_task.await.unwrap_err().is_cancelled());
    deregister_gate.add_permits(1);
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::DeregisterFinished("explicit-cleanup")
    );
    assert_eq!(
        recv_registry_event(&mut events).await,
        RegistryEvent::RegistryDropped("explicit-cleanup")
    );
    assert_eq!(deregister_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn blocked_active_request_forces_a_bounded_shutdown_timeout() {
    let address = available_address();
    let (request_started_sender, request_started_receiver) = oneshot::channel();
    let request_release = Arc::new(Semaphore::new(0));
    let mut config = ServerConfig::new(address);
    config.graceful_shutdown_timeout = Duration::from_millis(40);
    config.request_timeout = Duration::from_secs(5);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = Server::bind(address)
        .config(config)
        .service(ShutdownServiceImpl {
            started: Mutex::new(Some(request_started_sender)),
            release: request_release,
        });
    let server_task = tokio::spawn(server.run_with_shutdown(async move {
        let _ = shutdown_receiver.await;
    }));
    wait_until_listening(address).await;

    let runtime = ClientRuntime::builder().build().unwrap();
    let client = ShutdownServiceClient::builder(&runtime)
        .direct(format!("http://{address}"))
        .protocol(WireProtocol::SpringCloud)
        .connect()
        .await
        .unwrap();
    let request_task = tokio::spawn(async move { client.wait_for_release().await });
    tokio::time::timeout(Duration::from_secs(2), request_started_receiver)
        .await
        .expect("request did not start before its test deadline")
        .expect("request start signal was dropped");

    shutdown_sender.send(()).unwrap();
    assert!(matches!(
        await_server(server_task).await,
        Err(FusenError::Timeout(_))
    ));
    let request_result = tokio::time::timeout(Duration::from_secs(2), request_task)
        .await
        .expect("aborted request did not finish before its test deadline")
        .expect("request task panicked");
    assert!(request_result.is_err());
    runtime.shutdown().await.unwrap();
}

#[cfg(unix)]
const UNIX_SIGNAL_HELPER_TEST: &str = "protocol::protocol_e2e::unix_signal_server_helper";
#[cfg(unix)]
const UNIX_SIGNAL_HELPER_MARKER: &str = "FUSEN_TEST_UNIX_SIGNAL_HELPER";
#[cfg(unix)]
const UNIX_SIGNAL_HELPER_ADDRESS: &str = "FUSEN_TEST_UNIX_SIGNAL_ADDRESS";

#[cfg(unix)]
#[tokio::test]
async fn unix_signal_server_helper() {
    if std::env::var(UNIX_SIGNAL_HELPER_MARKER).ok().as_deref() != Some("1") {
        return;
    }
    let address = std::env::var(UNIX_SIGNAL_HELPER_ADDRESS)
        .expect("signal helper address was not provided")
        .parse::<SocketAddr>()
        .expect("signal helper address was invalid");
    Server::bind(address)
        .service(ProtocolServiceImpl)
        .run()
        .await
        .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn sigint_stops_the_default_server_runner() {
    unix_signal_stops_the_default_server_runner("INT").await;
}

#[cfg(unix)]
#[tokio::test]
async fn sigterm_stops_the_default_server_runner() {
    unix_signal_stops_the_default_server_runner("TERM").await;
}

#[cfg(unix)]
async fn unix_signal_stops_the_default_server_runner(signal: &str) {
    let address = available_address();
    let mut child = UnixSignalChild::spawn(address);
    let client_config = ClientConfig {
        connect_timeout: Duration::from_millis(100),
        request_timeout: Duration::from_millis(200),
        ..ClientConfig::default()
    };
    let runtime = ClientRuntime::builder()
        .config(client_config)
        .build()
        .unwrap();
    let client = ProtocolServiceClient::builder(&runtime)
        .direct(format!("http://{address}"))
        .protocol(WireProtocol::SpringCloud)
        .connect()
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.try_wait().expect("failed to poll signal helper") {
                panic!("signal helper exited before accepting requests: {status}");
            }
            match client.ping().await {
                Ok(response) => {
                    assert_eq!(response, "pong");
                    break;
                }
                Err(_) => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("signal helper did not accept requests before its test deadline");
    drop(client);
    runtime.shutdown().await.unwrap();

    assert!(
        child
            .try_wait()
            .expect("failed to poll signal helper before signaling")
            .is_none(),
        "signal helper exited before SIG{signal} was sent"
    );
    let signal_status = std::process::Command::new("kill")
        .args(["-s", signal])
        .arg(child.id().to_string())
        .status()
        .expect("failed to execute kill for signal helper");
    assert!(signal_status.success(), "kill failed for signal {signal}");

    let exit_status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = child.try_wait().expect("failed to poll signal helper") {
                break status;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("signal helper did not exit before its test deadline");
    assert!(
        exit_status.success(),
        "signal helper exited unsuccessfully after SIG{signal}: {exit_status}"
    );
}

#[cfg(unix)]
struct UnixSignalChild {
    child: Option<std::process::Child>,
}

#[cfg(unix)]
impl UnixSignalChild {
    fn spawn(address: SocketAddr) -> Self {
        let child = std::process::Command::new(
            std::env::current_exe().expect("failed to locate the test executable"),
        )
        .args(["--exact", UNIX_SIGNAL_HELPER_TEST, "--nocapture"])
        .env(UNIX_SIGNAL_HELPER_MARKER, "1")
        .env(UNIX_SIGNAL_HELPER_ADDRESS, address.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to start signal helper process");
        Self { child: Some(child) }
    }

    fn id(&self) -> u32 {
        self.child
            .as_ref()
            .expect("signal helper was already reaped")
            .id()
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let status = match self.child.as_mut() {
            Some(child) => child.try_wait()?,
            None => return Ok(None),
        };
        if status.is_some() {
            self.child.take();
        }
        Ok(status)
    }
}

#[cfg(unix)]
impl Drop for UnixSignalChild {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn registered_server_config(address: SocketAddr, graceful_timeout: Duration) -> ServerConfig {
    let mut config = ServerConfig::new(address);
    config.advertised_base_url = Some(format!("http://{address}"));
    config.graceful_shutdown_timeout = graceful_timeout;
    config.registry_timeout = Duration::from_secs(5);
    config
}

async fn recv_registry_event(events: &mut mpsc::UnboundedReceiver<RegistryEvent>) -> RegistryEvent {
    tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("registry event exceeded its test deadline")
        .expect("registry event channel closed")
}

async fn await_server(
    task: tokio::task::JoinHandle<Result<(), FusenError>>,
) -> Result<(), FusenError> {
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("server shutdown exceeded its test deadline")
        .expect("server task panicked")
}

fn available_address() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

async fn wait_until_listening(address: SocketAddr) {
    for _ in 0..100 {
        if TcpStream::connect(address).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("server did not start listening");
}
