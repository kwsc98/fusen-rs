//! Deterministic Server/Registry startup, rollback, and shutdown coverage.

use fusen_register::{
    RegistrationHandle, RegistrationRequest, Registry, SubscriptionHandle, SubscriptionRequest,
    error::RegistryError, provider,
};
use fusen_rs::{
    ClientRuntime, Error, Response, Server, ServerConfig, ServerErrorKind, ServerRegistryConfig,
    ServerState, interface,
};
use std::{
    future::pending,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Barrier, Semaphore, oneshot};

#[interface(name = "alpha-registry-e2e")]
trait AlphaRegistryService {
    #[fusen_rs::method(method = "GET", path = "/registry/alpha")]
    async fn call(&self) -> Result<Response<String>, Error>;
}

#[interface(name = "zeta-registry-e2e")]
trait ZetaRegistryService {
    #[fusen_rs::method(method = "GET", path = "/registry/zeta")]
    async fn call(&self) -> Result<Response<String>, Error>;
}

struct RegistryServiceImpl;

struct BlockingRegistryServiceImpl {
    entered: Arc<Barrier>,
    release: Arc<Semaphore>,
}

impl AlphaRegistryService for RegistryServiceImpl {
    async fn call(&self) -> Result<Response<String>, Error> {
        Ok(Response::new("alpha".into()))
    }
}

impl ZetaRegistryService for RegistryServiceImpl {
    async fn call(&self) -> Result<Response<String>, Error> {
        Ok(Response::new("zeta".into()))
    }
}

impl AlphaRegistryService for BlockingRegistryServiceImpl {
    async fn call(&self) -> Result<Response<String>, Error> {
        self.entered.wait().await;
        let _permit = self
            .release
            .acquire()
            .await
            .expect("test service release remains open");
        Ok(Response::new("drained".into()))
    }
}

struct CoordinatedCloseRegistry {
    close_started: Mutex<Option<oneshot::Sender<()>>>,
    close_release: Mutex<Option<oneshot::Receiver<()>>>,
}

impl Registry for CoordinatedCloseRegistry {
    fn prepare_registration(
        &self,
        _request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        let close_started = self
            .close_started
            .lock()
            .unwrap()
            .take()
            .expect("test registry prepares one registration");
        let close_release = self
            .close_release
            .lock()
            .unwrap()
            .take()
            .expect("test registry has one close release");
        Ok(provider::registration(
            async { Ok(()) },
            move || async move {
                let _ = close_started.send(());
                let _ = close_release.await;
                Ok(())
            },
        ))
    }

    fn prepare_subscription(
        &self,
        _request: SubscriptionRequest,
    ) -> Result<SubscriptionHandle, RegistryError> {
        Err(RegistryError::message(
            fusen_register::error::RegistryOperation::PrepareSubscription,
            fusen_register::error::RegistryErrorKind::InvalidResource,
            "test registry does not support subscriptions",
        ))
    }
}

#[derive(Clone, Default)]
struct FakeBehavior {
    fail_activate: Option<Arc<str>>,
    fail_close: Option<Arc<str>>,
    pending_close: Option<Arc<str>>,
}

struct FakeRegistry {
    name: Arc<str>,
    events: Arc<Mutex<Vec<String>>>,
    behavior: FakeBehavior,
}

impl FakeRegistry {
    fn new(name: &str, events: Arc<Mutex<Vec<String>>>, behavior: FakeBehavior) -> Self {
        Self {
            name: Arc::from(name),
            events,
            behavior,
        }
    }
}

impl Registry for FakeRegistry {
    fn prepare_registration(
        &self,
        request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        let registration = request.into_registration();
        let identity: Arc<str> = Arc::from(registration.selector().identity());
        let key = format!("{}:{}", self.name, identity);
        push(&self.events, format!("prepare:{key}"));

        let activation_events = self.events.clone();
        let activation_key = key.clone();
        let activation_identity = identity.clone();
        let fail_activate = self.behavior.fail_activate.clone();
        let close_events = self.events.clone();
        let close_identity = identity;
        let fail_close = self.behavior.fail_close.clone();
        let pending_close = self.behavior.pending_close.clone();
        Ok(provider::registration(
            async move {
                push(&activation_events, format!("activate:{activation_key}"));
                if fail_activate.as_deref() == Some(activation_identity.as_ref()) {
                    Err(registry_error("activation failed"))
                } else {
                    Ok(())
                }
            },
            move || async move {
                push(&close_events, format!("close:{key}"));
                if pending_close.as_deref() == Some(close_identity.as_ref()) {
                    pending::<()>().await;
                }
                if fail_close.as_deref() == Some(close_identity.as_ref()) {
                    Err(registry_error("close failed"))
                } else {
                    Ok(())
                }
            },
        ))
    }

    fn prepare_subscription(
        &self,
        _request: SubscriptionRequest,
    ) -> Result<SubscriptionHandle, RegistryError> {
        Err(RegistryError::message(
            fusen_register::error::RegistryOperation::PrepareSubscription,
            fusen_register::error::RegistryErrorKind::InvalidResource,
            "test registry does not support subscriptions",
        ))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registrations_follow_stable_order_and_close_in_reverse() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let config = server_config(Duration::from_secs(1));
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .registry(
            "first",
            FakeRegistry::new("first", events.clone(), FakeBehavior::default()),
        )
        .registry(
            "second",
            FakeRegistry::new("second", events.clone(), FakeBehavior::default()),
        )
        .interface(ZetaRegistryServiceServer::new(RegistryServiceImpl))
        .interface(AlphaRegistryServiceServer::new(RegistryServiceImpl))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();

    server.shutdown().await.unwrap();
    let events = snapshot(&events);
    let keys = expected_keys(&["first", "second"]);
    let expected = keys
        .iter()
        .map(|key| format!("prepare:{key}"))
        .chain(keys.iter().map(|key| format!("activate:{key}")))
        .chain(keys.iter().rev().map(|key| format!("close:{key}")))
        .collect::<Vec<_>>();
    assert_eq!(events, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_closes_listener_before_registry_and_connection_drain_in_parallel() {
    let entered = Arc::new(Barrier::new(2));
    let service_release = Arc::new(Semaphore::new(0));
    let (close_started_sender, close_started) = oneshot::channel();
    let (close_release, close_release_receiver) = oneshot::channel();
    let registry = CoordinatedCloseRegistry {
        close_started: Mutex::new(Some(close_started_sender)),
        close_release: Mutex::new(Some(close_release_receiver)),
    };
    let config = ServerConfig::builder()
        .registry(
            ServerRegistryConfig::builder()
                .startup_timeout(Duration::from_secs(5))
                .operation_timeout(Duration::from_secs(5))
                .max_concurrent_operations(1)
                .build()
                .unwrap(),
        )
        .graceful_shutdown_timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .registry("coordinated", registry)
        .interface(AlphaRegistryServiceServer::new(
            BlockingRegistryServiceImpl {
                entered: entered.clone(),
                release: service_release.clone(),
            },
        ))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let address = server.local_addr();
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = AlphaRegistryServiceClient::builder(&runtime)
        .direct(format!("http://{address}"))
        .connect()
        .await
        .unwrap();
    let invocation = tokio::spawn(async move { client.call().await });
    entered.wait().await;

    let handle = server.handle();
    let shutdown = tokio::spawn(async move { handle.shutdown().await });
    wait_for_state(&server, ServerState::Draining).await;
    tokio::time::timeout(Duration::from_secs(1), close_started)
        .await
        .expect("registry close must start while the request remains in flight")
        .unwrap();
    assert!(
        !shutdown.is_finished(),
        "shutdown must wait for both registry and connection drain"
    );

    service_release.add_permits(1);
    close_release.send(()).unwrap();
    assert_eq!(invocation.await.unwrap().unwrap().into_body(), "drained");
    shutdown.await.unwrap().unwrap();
    server.wait().await.unwrap();
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activation_failure_rolls_back_every_started_handle_in_reverse() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let behavior = FakeBehavior {
        fail_activate: Some(Arc::from("zeta-registry-e2e")),
        ..FakeBehavior::default()
    };
    let result = Server::builder("127.0.0.1:0")
        .config(server_config(Duration::from_secs(1)))
        .registry(
            "registry",
            FakeRegistry::new("registry", events.clone(), behavior),
        )
        .interface(ZetaRegistryServiceServer::new(RegistryServiceImpl))
        .interface(AlphaRegistryServiceServer::new(RegistryServiceImpl))
        .build()
        .unwrap()
        .start()
        .await;
    let error = match result {
        Ok(_) => panic!("activation failure must prevent Ready"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ServerErrorKind::Registry);

    assert_eq!(
        snapshot(&events),
        [
            "prepare:registry:alpha-registry-e2e",
            "prepare:registry:zeta-registry-e2e",
            "activate:registry:alpha-registry-e2e",
            "activate:registry:zeta-registry-e2e",
            "close:registry:zeta-registry-e2e",
            "close:registry:alpha-registry-e2e",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rollback_provider_error_does_not_replace_the_startup_failure() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let behavior = FakeBehavior {
        fail_activate: Some(Arc::from("zeta-registry-e2e")),
        fail_close: Some(Arc::from("zeta-registry-e2e")),
        ..FakeBehavior::default()
    };
    let result = Server::builder("127.0.0.1:0")
        .config(server_config(Duration::from_secs(1)))
        .registry(
            "registry",
            FakeRegistry::new("registry", events.clone(), behavior),
        )
        .interface(ZetaRegistryServiceServer::new(RegistryServiceImpl))
        .interface(AlphaRegistryServiceServer::new(RegistryServiceImpl))
        .build()
        .unwrap()
        .start()
        .await;
    let error = match result {
        Ok(_) => panic!("activation failure must prevent Ready"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ServerErrorKind::Registry);
    assert!(
        error.message().contains("activate registration"),
        "the original activation failure must remain observable: {error}"
    );
    assert_eq!(
        close_events(&events),
        [
            "close:registry:zeta-registry-e2e",
            "close:registry:alpha-registry-e2e",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_provider_error_does_not_skip_remaining_handles() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let behavior = FakeBehavior {
        fail_close: Some(Arc::from("zeta-registry-e2e")),
        ..FakeBehavior::default()
    };
    let server = one_registry_server(events.clone(), behavior, Duration::from_secs(1)).await;
    let error = server.shutdown().await.unwrap_err();
    assert_eq!(error.kind(), ServerErrorKind::Registry);
    assert_eq!(
        close_events(&events),
        [
            "close:registry:zeta-registry-e2e",
            "close:registry:alpha-registry-e2e",
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_close_operation_timeout_has_timeout_priority_and_continues_cleanup() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let behavior = FakeBehavior {
        pending_close: Some(Arc::from("zeta-registry-e2e")),
        ..FakeBehavior::default()
    };
    let server = one_registry_server(events.clone(), behavior, Duration::from_millis(20)).await;
    let error = tokio::time::timeout(Duration::from_secs(1), server.shutdown())
        .await
        .expect("shutdown must remain bounded")
        .unwrap_err();
    assert_eq!(error.kind(), ServerErrorKind::Timeout);
    assert_eq!(
        close_events(&events),
        [
            "close:registry:zeta-registry-e2e",
            "close:registry:alpha-registry-e2e",
        ]
    );
}

async fn one_registry_server(
    events: Arc<Mutex<Vec<String>>>,
    behavior: FakeBehavior,
    operation_timeout: Duration,
) -> fusen_rs::RunningServer {
    Server::builder("127.0.0.1:0")
        .config(server_config(operation_timeout))
        .registry("registry", FakeRegistry::new("registry", events, behavior))
        .interface(ZetaRegistryServiceServer::new(RegistryServiceImpl))
        .interface(AlphaRegistryServiceServer::new(RegistryServiceImpl))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap()
}

fn server_config(operation_timeout: Duration) -> ServerConfig {
    ServerConfig::builder()
        .registry(
            ServerRegistryConfig::builder()
                .startup_timeout(Duration::from_secs(1))
                .operation_timeout(operation_timeout)
                .max_concurrent_operations(1)
                .build()
                .unwrap(),
        )
        .graceful_shutdown_timeout(Duration::from_secs(1))
        .build()
        .unwrap()
}

fn expected_keys(registries: &[&str]) -> Vec<String> {
    let mut keys = Vec::new();
    for registry in registries {
        for service in ["alpha-registry-e2e", "zeta-registry-e2e"] {
            keys.push(format!("{registry}:{service}"));
        }
    }
    keys
}

fn close_events(events: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    snapshot(events)
        .into_iter()
        .filter(|event| event.starts_with("close:"))
        .collect()
}

fn push(events: &Arc<Mutex<Vec<String>>>, event: String) {
    events.lock().unwrap().push(event);
}

fn snapshot(events: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    events.lock().unwrap().clone()
}

fn registry_error(message: &str) -> RegistryError {
    RegistryError::message(
        fusen_register::error::RegistryOperation::CloseRegistration,
        fusen_register::error::RegistryErrorKind::Unavailable,
        message,
    )
}

async fn wait_for_state(server: &fusen_rs::RunningServer, expected: ServerState) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while server.state() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("server lifecycle state must advance");
}
