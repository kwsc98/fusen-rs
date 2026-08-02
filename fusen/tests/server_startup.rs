//! Deterministic coverage for the server's pre-Ready and cancelled-startup states.

use fusen_register::{
    RegistrationHandle, RegistrationRequest, Registry, SubscriptionHandle, SubscriptionRequest,
    error::RegistryError, provider,
};
use fusen_rs::{Error, Response, Server, ServerConfig, ServerRegistryConfig, interface};
use serde_json::Value;
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
    sync::oneshot,
};

#[interface(name = "startup-lifecycle-e2e")]
trait StartupLifecycleService {
    #[fusen_rs::method(method = "GET", path = "/startup")]
    async fn check(&self) -> Result<Response<String>, Error>;
}

struct StartupLifecycleServiceImpl;

impl StartupLifecycleService for StartupLifecycleServiceImpl {
    async fn check(&self) -> Result<Response<String>, Error> {
        Ok(Response::new("ready".to_owned()))
    }
}

struct GatedRegistry {
    endpoint: Mutex<Option<oneshot::Sender<SocketAddr>>>,
    activation_started: Mutex<Option<oneshot::Sender<()>>>,
    activation_release: Mutex<Option<oneshot::Receiver<()>>>,
    cleanup_completed: Mutex<Option<oneshot::Sender<()>>>,
    registry_dropped: Mutex<Option<oneshot::Sender<()>>>,
    cleanup_count: Arc<AtomicUsize>,
}

struct RegistrySignals {
    endpoint: oneshot::Receiver<SocketAddr>,
    activation_started: oneshot::Receiver<()>,
    activation_release: oneshot::Sender<()>,
    cleanup_completed: oneshot::Receiver<()>,
    registry_dropped: oneshot::Receiver<()>,
    cleanup_count: Arc<AtomicUsize>,
}

impl GatedRegistry {
    fn controlled() -> (Self, RegistrySignals) {
        let (endpoint_sender, endpoint) = oneshot::channel();
        let (activation_started_sender, activation_started) = oneshot::channel();
        let (activation_release, activation_release_receiver) = oneshot::channel();
        let (cleanup_completed_sender, cleanup_completed) = oneshot::channel();
        let (registry_dropped_sender, registry_dropped) = oneshot::channel();
        let cleanup_count = Arc::new(AtomicUsize::new(0));
        (
            Self {
                endpoint: Mutex::new(Some(endpoint_sender)),
                activation_started: Mutex::new(Some(activation_started_sender)),
                activation_release: Mutex::new(Some(activation_release_receiver)),
                cleanup_completed: Mutex::new(Some(cleanup_completed_sender)),
                registry_dropped: Mutex::new(Some(registry_dropped_sender)),
                cleanup_count: cleanup_count.clone(),
            },
            RegistrySignals {
                endpoint,
                activation_started,
                activation_release,
                cleanup_completed,
                registry_dropped,
                cleanup_count,
            },
        )
    }
}

impl Registry for GatedRegistry {
    fn prepare_registration(
        &self,
        request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        let registration = request.registration();
        let address = registration
            .endpoint()
            .as_url()
            .socket_addrs(|| None)
            .expect("test endpoint resolves without network access")
            .into_iter()
            .next()
            .expect("test endpoint contains a socket address");
        self.endpoint
            .lock()
            .unwrap()
            .take()
            .expect("test prepares exactly one registration")
            .send(address)
            .expect("test endpoint receiver remains alive");
        let activation_started = self
            .activation_started
            .lock()
            .unwrap()
            .take()
            .expect("test activation is prepared once");
        let activation_release = self
            .activation_release
            .lock()
            .unwrap()
            .take()
            .expect("test activation release is present");
        let cleanup_completed = self
            .cleanup_completed
            .lock()
            .unwrap()
            .take()
            .expect("test cleanup is prepared once");
        let cleanup_count = self.cleanup_count.clone();

        Ok(provider::registration(
            async move {
                let _ = activation_started.send(());
                let _ = activation_release.await;
                Ok(())
            },
            move || async move {
                cleanup_count.fetch_add(1, Ordering::SeqCst);
                let _ = cleanup_completed.send(());
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
            "startup test registry does not support subscriptions",
        ))
    }
}

impl Drop for GatedRegistry {
    fn drop(&mut self) {
        if let Some(completed) = self.registry_dropped.lock().unwrap().take() {
            let _ = completed.send(());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn requests_before_registration_ready_fail_without_reading_the_body() {
    let (registry, signals) = GatedRegistry::controlled();
    let RegistrySignals {
        endpoint,
        activation_started,
        activation_release,
        cleanup_completed,
        registry_dropped,
        cleanup_count,
    } = signals;
    let server = build_server(registry);
    let startup = tokio::spawn(async move { server.start().await });
    let address = endpoint.await.unwrap();
    activation_started.await.unwrap();
    assert!(
        !startup.is_finished(),
        "start must wait for registration Ready"
    );

    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            format!(
                "GET /startup HTTP/1.1\r\nHost: {address}\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), stream.read_to_end(&mut response))
        .await
        .expect("not-ready response must not wait for the unsent request body")
        .unwrap();
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 503 Service Unavailable\r\n"));
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    let problem: Value = serde_json::from_str(body).unwrap();
    assert_eq!(problem["code"], "not_ready");
    assert_eq!(problem["retryable"], false);
    assert!(
        !startup.is_finished(),
        "request handling must not publish Ready"
    );

    activation_release.send(()).unwrap();
    let running = startup.await.unwrap().unwrap();
    running.shutdown().await.unwrap();
    cleanup_completed.await.unwrap();
    registry_dropped.await.unwrap();
    assert_eq!(cleanup_count.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborting_start_compensates_a_late_registration_success_exactly_once() {
    let (registry, signals) = GatedRegistry::controlled();
    let RegistrySignals {
        endpoint,
        activation_started,
        activation_release,
        cleanup_completed,
        registry_dropped,
        cleanup_count,
    } = signals;
    let server = build_server(registry);
    let startup = tokio::spawn(async move { server.start().await });
    endpoint.await.unwrap();
    activation_started.await.unwrap();

    startup.abort();
    let join_error = match startup.await {
        Err(error) => error,
        Ok(_) => panic!("aborted Server::start task must not complete normally"),
    };
    assert!(join_error.is_cancelled());

    // Coordinator completion is the portable listener-close barrier; a negative TCP probe can
    // keep succeeding on Windows for connections queued before the listening socket is dropped.
    activation_release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), cleanup_completed)
        .await
        .expect("late activation success must be compensated")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), registry_dropped)
        .await
        .expect("cancelled startup coordinator must terminate")
        .unwrap();
    assert_eq!(cleanup_count.load(Ordering::SeqCst), 1);
}

fn build_server(registry: GatedRegistry) -> Server {
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
    Server::builder("127.0.0.1:0")
        .config(config)
        .registry("gated", registry)
        .interface(StartupLifecycleServiceServer::new(
            StartupLifecycleServiceImpl,
        ))
        .build()
        .unwrap()
}
