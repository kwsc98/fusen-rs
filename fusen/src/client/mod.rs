mod builder;
/// Pluggable client-side routing and load-balancing APIs.
pub(crate) mod cluster;
mod invocation;
mod runtime;
mod subscription;

#[doc(hidden)]
pub use builder::ServiceClientBuilder;
#[doc(hidden)]
pub use invocation::ServiceClient;
pub use runtime::{
    ClientConfig, ClientRuntime, ClientRuntimeBuilder, Http1PoolConfig, Http2PoolConfig,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FusenError, InvocationObserver, Middleware, Next, RpcContext, RpcResult,
        invocation::{InvocationFinish, InvocationOutcome, InvocationPhase, InvocationStart},
    };
    use fusen_contract::{
        MethodDescriptor, MethodId, ServiceDescriptor, ServiceInstance, ServiceRegistration,
        ServiceSelector, ServiceWeight, StaticBoxFuture, WireProtocol,
    };
    use fusen_register::{
        Register, ServiceSubscription, directory::Directory, error::RegisterError,
        subscription_cleanup,
    };
    use http::Method;
    use std::{
        sync::{
            Arc, Mutex, OnceLock,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    #[derive(Clone, Default)]
    struct RecordingObserver(Arc<Mutex<Vec<(InvocationOutcome, InvocationPhase)>>>);

    impl InvocationObserver for RecordingObserver {
        fn on_start(&self, _event: &InvocationStart<'_>) {}

        fn on_finish(&self, event: &InvocationFinish<'_>) {
            self.0.lock().unwrap().push((event.outcome, event.phase));
        }
    }

    fn service() -> &'static ServiceDescriptor {
        static SERVICE: OnceLock<ServiceDescriptor> = OnceLock::new();
        SERVICE.get_or_init(|| {
            ServiceDescriptor::__new(
                "runtime-test",
                None,
                None,
                vec![
                    MethodDescriptor::__new(
                        MethodId::__new(0),
                        "call",
                        Method::POST,
                        "/call",
                        Vec::new(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap()
        })
    }

    fn service_named(name: impl Into<String>) -> &'static ServiceDescriptor {
        let name = name.into();
        Box::leak(Box::new(
            ServiceDescriptor::__new(
                name,
                None,
                None,
                vec![
                    MethodDescriptor::__new(
                        MethodId::__new(0),
                        "call",
                        Method::POST,
                        "/call",
                        Vec::new(),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        ))
    }

    #[test]
    fn validates_client_connection_pool_settings() {
        let mut config = ClientConfig::default();
        config.http2_pool.connections_per_host = 0;
        assert!(matches!(
            ClientRuntime::builder().config(config).build(),
            Err(FusenError::InvalidRequest(_))
        ));

        let mut config = ClientConfig::default();
        config.http1_pool.idle_timeout = Some(Duration::ZERO);
        assert!(matches!(
            ClientRuntime::builder().config(config).build(),
            Err(FusenError::InvalidRequest(_))
        ));

        let config = Http1PoolConfig {
            max_idle_per_host: 0,
            ..Http1PoolConfig::default()
        };
        assert!(ClientRuntime::builder().http1_pool(config).build().is_ok());
    }

    #[derive(Clone)]
    struct CountingRegister {
        subscriptions: Arc<AtomicUsize>,
        cleanups: Arc<AtomicUsize>,
    }

    impl Register for CountingRegister {
        fn register(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn deregister(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn subscribe(
            &self,
            _selector: ServiceSelector,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<ServiceSubscription, RegisterError>> {
            self.subscriptions.fetch_add(1, Ordering::SeqCst);
            let cleanups = self.cleanups.clone();
            Box::pin(async move {
                let (closer, cleanup) = subscription_cleanup();
                tokio::spawn(cleanup.run(async move {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }));
                Ok(ServiceSubscription::new(
                    Directory::fixed(vec![ServiceInstance::new(
                        "http://127.0.0.1:1".parse().unwrap(),
                        ServiceWeight::default(),
                    )]),
                    closer,
                ))
            })
        }
    }

    #[tokio::test]
    async fn shutdown_is_idempotent_and_closes_all_shared_subscriptions() {
        let cleanups = Arc::new(AtomicUsize::new(0));
        let subscriptions = Arc::new(AtomicUsize::new(0));
        let runtime = ClientRuntime::builder()
            .registry(CountingRegister {
                subscriptions: subscriptions.clone(),
                cleanups: cleanups.clone(),
            })
            .build()
            .unwrap();
        let _first = runtime
            .__client_builder(service())
            .discover()
            .connect()
            .await
            .unwrap();
        let _second = runtime
            .__client_builder(service())
            .discover()
            .connect()
            .await
            .unwrap();
        runtime.shutdown().await.unwrap();
        runtime.shutdown().await.unwrap();
        assert_eq!(subscriptions.load(Ordering::SeqCst), 1);
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_connects_share_subscription_until_the_last_lease_drops() {
        let subscriptions = Arc::new(AtomicUsize::new(0));
        let cleanups = Arc::new(AtomicUsize::new(0));
        let runtime = ClientRuntime::builder()
            .registry(CountingRegister {
                subscriptions: subscriptions.clone(),
                cleanups: cleanups.clone(),
            })
            .build()
            .unwrap();
        let first = runtime.__client_builder(service()).discover().connect();
        let second = runtime.__client_builder(service()).discover().connect();
        let (first, second) = tokio::join!(first, second);
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(subscriptions.load(Ordering::SeqCst), 1);

        drop(first);
        tokio::task::yield_now().await;
        assert_eq!(cleanups.load(Ordering::SeqCst), 0);
        drop(second);
        tokio::time::timeout(Duration::from_secs(1), async {
            while cleanups.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        runtime.shutdown().await.unwrap();
    }

    #[derive(Clone)]
    struct FailingSubscribeRegister {
        subscriptions: Arc<AtomicUsize>,
        started: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    impl Register for FailingSubscribeRegister {
        fn register(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn deregister(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn subscribe(
            &self,
            _selector: ServiceSelector,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<ServiceSubscription, RegisterError>> {
            self.subscriptions.fetch_add(1, Ordering::SeqCst);
            let started = self.started.clone();
            let release = self.release.clone();
            Box::pin(async move {
                started.notify_one();
                release.notified().await;
                Err(RegisterError::InvalidResource(
                    "expected subscription failure".into(),
                ))
            })
        }
    }

    #[tokio::test]
    async fn concurrent_connects_share_one_creation_failure() {
        let subscriptions = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let runtime = ClientRuntime::builder()
            .registry(FailingSubscribeRegister {
                subscriptions: subscriptions.clone(),
                started: started.clone(),
                release: release.clone(),
            })
            .build()
            .unwrap();
        let first = tokio::spawn({
            let runtime = runtime.clone();
            async move {
                runtime
                    .__client_builder(service())
                    .discover()
                    .connect()
                    .await
            }
        });
        let second = tokio::spawn({
            let runtime = runtime.clone();
            async move {
                runtime
                    .__client_builder(service())
                    .discover()
                    .connect()
                    .await
            }
        });
        started.notified().await;
        release.notify_waiters();

        let Err(first) = first.await.unwrap() else {
            panic!("first connection unexpectedly succeeded");
        };
        let Err(second) = second.await.unwrap() else {
            panic!("second connection unexpectedly succeeded");
        };
        assert_eq!(first.to_string(), second.to_string());
        assert_eq!(subscriptions.load(Ordering::SeqCst), 1);
        runtime.shutdown().await.unwrap();
    }

    #[test]
    fn dropping_last_lease_outside_runtime_still_schedules_cleanup() {
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let subscriptions = Arc::new(AtomicUsize::new(0));
        let cleanups = Arc::new(AtomicUsize::new(0));
        let (runtime, client) = executor.block_on(async {
            let runtime = ClientRuntime::builder()
                .registry(CountingRegister {
                    subscriptions,
                    cleanups: cleanups.clone(),
                })
                .build()
                .unwrap();
            let client = runtime
                .__client_builder(service())
                .discover()
                .connect()
                .await
                .unwrap();
            (runtime, client)
        });

        drop(client);
        executor
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(1), async {
                    while cleanups.load(Ordering::SeqCst) == 0 {
                        tokio::task::yield_now().await;
                    }
                })
                .await
            })
            .unwrap();
        executor.block_on(runtime.shutdown()).unwrap();
    }

    #[derive(Clone)]
    struct GatedSubscribeRegister {
        started: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        cleanups: Arc<AtomicUsize>,
    }

    impl Register for GatedSubscribeRegister {
        fn register(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn deregister(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn subscribe(
            &self,
            _selector: ServiceSelector,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<ServiceSubscription, RegisterError>> {
            let started = self.started.clone();
            let release = self.release.clone();
            let cleanups = self.cleanups.clone();
            Box::pin(async move {
                started.notify_one();
                release.notified().await;
                let (closer, cleanup) = subscription_cleanup();
                tokio::spawn(cleanup.run(async move {
                    cleanups.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }));
                Ok(ServiceSubscription::new(
                    Directory::fixed(Vec::new()),
                    closer,
                ))
            })
        }
    }

    #[tokio::test]
    async fn connect_finishing_after_shutdown_closes_its_untracked_subscription() {
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let cleanups = Arc::new(AtomicUsize::new(0));
        let runtime = ClientRuntime::builder()
            .registry(GatedSubscribeRegister {
                started: started.clone(),
                release: release.clone(),
                cleanups: cleanups.clone(),
            })
            .build()
            .unwrap();
        let connecting = tokio::spawn({
            let runtime = runtime.clone();
            async move {
                runtime
                    .__client_builder(service())
                    .discover()
                    .connect()
                    .await
            }
        });
        started.notified().await;

        let shutdown = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.shutdown().await }
        });
        release.notify_one();
        shutdown.await.unwrap().unwrap();
        assert!(matches!(
            connecting.await.unwrap(),
            Err(FusenError::ServiceUnavailable(_))
        ));
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn aborting_the_only_connect_closes_the_created_subscription() {
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let cleanups = Arc::new(AtomicUsize::new(0));
        let runtime = ClientRuntime::builder()
            .registry(GatedSubscribeRegister {
                started: started.clone(),
                release: release.clone(),
                cleanups: cleanups.clone(),
            })
            .build()
            .unwrap();
        let connecting = tokio::spawn({
            let runtime = runtime.clone();
            async move {
                runtime
                    .__client_builder(service())
                    .discover()
                    .connect()
                    .await
            }
        });
        started.notified().await;
        connecting.abort();
        assert!(matches!(connecting.await, Err(error) if error.is_cancelled()));
        release.notify_one();

        tokio::time::timeout(Duration::from_secs(1), async {
            while cleanups.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        runtime.shutdown().await.unwrap();
        assert_eq!(cleanups.load(Ordering::SeqCst), 1);
    }

    #[derive(Clone)]
    enum CleanupMode {
        GateFirst(Arc<tokio::sync::Notify>),
        FailFirst,
    }

    #[derive(Clone)]
    struct ControlledCleanupRegister {
        mode: CleanupMode,
        subscriptions: Arc<AtomicUsize>,
        started: Arc<AtomicUsize>,
        completed: Arc<AtomicUsize>,
    }

    impl Register for ControlledCleanupRegister {
        fn register(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn deregister(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn subscribe(
            &self,
            _selector: ServiceSelector,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<ServiceSubscription, RegisterError>> {
            let index = self.subscriptions.fetch_add(1, Ordering::SeqCst);
            let mode = self.mode.clone();
            let started = self.started.clone();
            let completed = self.completed.clone();
            Box::pin(async move {
                let (closer, cleanup) = subscription_cleanup();
                tokio::spawn(cleanup.run(async move {
                    started.fetch_add(1, Ordering::SeqCst);
                    match mode {
                        CleanupMode::GateFirst(release) if index == 0 => {
                            release.notified().await;
                        }
                        CleanupMode::FailFirst if index == 0 => {
                            return Err(RegisterError::InvalidResource(
                                "expected cleanup failure".into(),
                            ));
                        }
                        _ => {}
                    }
                    completed.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }));
                Ok(ServiceSubscription::new(
                    Directory::fixed(Vec::new()),
                    closer,
                ))
            })
        }
    }

    async fn connect_discovered_clients(runtime: &ClientRuntime, count: usize) {
        for index in 0..count {
            runtime
                .__client_builder(service_named(format!("runtime-test-{index}")))
                .discover()
                .connect()
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn shutdown_retries_timeouts_after_starting_every_cleanup() {
        let release = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let config = ClientConfig {
            subscription_close_timeout: Duration::from_millis(20),
            ..ClientConfig::default()
        };
        let runtime = ClientRuntime::builder()
            .config(config)
            .registry(ControlledCleanupRegister {
                mode: CleanupMode::GateFirst(release.clone()),
                subscriptions: Arc::new(AtomicUsize::new(0)),
                started: started.clone(),
                completed: completed.clone(),
            })
            .build()
            .unwrap();
        connect_discovered_clients(&runtime, 2).await;

        assert!(matches!(
            runtime.shutdown().await,
            Err(FusenError::Timeout(_))
        ));
        assert_eq!(started.load(Ordering::SeqCst), 2);
        assert_eq!(completed.load(Ordering::SeqCst), 1);

        release.notify_one();
        runtime.shutdown().await.unwrap();
        assert_eq!(started.load(Ordering::SeqCst), 2);
        assert_eq!(completed.load(Ordering::SeqCst), 2);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn aborted_shutdown_keeps_subscriptions_for_the_next_attempt() {
        let release = Arc::new(tokio::sync::Notify::new());
        let started = Arc::new(AtomicUsize::new(0));
        let config = ClientConfig {
            subscription_close_timeout: Duration::from_millis(20),
            ..ClientConfig::default()
        };
        let runtime = ClientRuntime::builder()
            .config(config)
            .registry(ControlledCleanupRegister {
                mode: CleanupMode::GateFirst(release.clone()),
                subscriptions: Arc::new(AtomicUsize::new(0)),
                started: started.clone(),
                completed: Arc::new(AtomicUsize::new(0)),
            })
            .build()
            .unwrap();
        connect_discovered_clients(&runtime, 1).await;

        let shutdown = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.shutdown().await }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while started.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        shutdown.abort();
        assert!(shutdown.await.unwrap_err().is_cancelled());

        assert!(matches!(
            runtime.shutdown().await,
            Err(FusenError::Timeout(_))
        ));
        release.notify_one();
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_records_terminal_error_after_all_cleanups_run() {
        let started = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let runtime = ClientRuntime::builder()
            .registry(ControlledCleanupRegister {
                mode: CleanupMode::FailFirst,
                subscriptions: Arc::new(AtomicUsize::new(0)),
                started: started.clone(),
                completed: completed.clone(),
            })
            .build()
            .unwrap();
        connect_discovered_clients(&runtime, 2).await;

        let first = runtime.shutdown().await.unwrap_err().to_string();
        let second = runtime.shutdown().await.unwrap_err().to_string();
        assert_eq!(started.load(Ordering::SeqCst), 2);
        assert_eq!(completed.load(Ordering::SeqCst), 1);
        assert_eq!(first, second);
        assert!(first.contains("expected cleanup failure"));
    }

    #[tokio::test]
    async fn concurrent_shutdown_calls_share_one_cleanup_pass() {
        let cleanups = Arc::new(AtomicUsize::new(0));
        let runtime = ClientRuntime::builder()
            .registry(CountingRegister {
                subscriptions: Arc::new(AtomicUsize::new(0)),
                cleanups: cleanups.clone(),
            })
            .build()
            .unwrap();
        connect_discovered_clients(&runtime, 2).await;

        let first = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.shutdown().await }
        });
        let second = tokio::spawn({
            let runtime = runtime.clone();
            async move { runtime.shutdown().await }
        });
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(cleanups.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn shutdown_rejects_existing_invocations_and_new_connections() {
        let observer = RecordingObserver::default();
        let runtime = ClientRuntime::builder()
            .observer(observer.clone())
            .build()
            .unwrap();
        let client = runtime
            .__client_builder(service())
            .direct("http://127.0.0.1:1")
            .connect()
            .await
            .unwrap();
        runtime.shutdown().await.unwrap();
        assert!(matches!(
            client.__invoke(MethodId::__new(0), Vec::new()).await,
            Err(FusenError::ServiceUnavailable(_))
        ));
        assert_eq!(
            *observer.0.lock().unwrap(),
            [(InvocationOutcome::Error, InvocationPhase::Admission)]
        );
        assert!(matches!(
            runtime
                .__client_builder(service())
                .direct("http://127.0.0.1:1")
                .connect()
                .await,
            Err(FusenError::ServiceUnavailable(_))
        ));
    }

    #[derive(Clone)]
    struct EmptyRegister;

    impl Register for EmptyRegister {
        fn register(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn deregister(
            &self,
            _resource: Arc<ServiceRegistration>,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<(), RegisterError>> {
            Box::pin(async { Ok(()) })
        }

        fn subscribe(
            &self,
            _selector: ServiceSelector,
            _protocol: WireProtocol,
        ) -> StaticBoxFuture<Result<ServiceSubscription, RegisterError>> {
            Box::pin(async { Ok(ServiceSubscription::local(Vec::new())) })
        }
    }

    #[tokio::test]
    async fn empty_discovery_reports_one_cluster_error() {
        let observer = RecordingObserver::default();
        let runtime = ClientRuntime::builder()
            .registry(EmptyRegister)
            .observer(observer.clone())
            .build()
            .unwrap();
        let client = runtime
            .__client_builder(service())
            .discover()
            .connect()
            .await
            .unwrap();
        assert!(matches!(
            client.__invoke(MethodId::__new(0), Vec::new()).await,
            Err(FusenError::ServiceUnavailable(_))
        ));
        assert_eq!(
            *observer.0.lock().unwrap(),
            [(InvocationOutcome::Error, InvocationPhase::Cluster)]
        );
        runtime.shutdown().await.unwrap();
    }

    struct PendingMiddleware;

    impl Middleware for PendingMiddleware {
        async fn handle<'a>(&'a self, _context: RpcContext, _next: Next<'a>) -> RpcResult {
            std::future::pending().await
        }
    }

    #[derive(Clone, Default)]
    struct CancellationObserver {
        started: Arc<tokio::sync::Notify>,
        outcomes: Arc<Mutex<Vec<(InvocationOutcome, InvocationPhase)>>>,
    }

    impl InvocationObserver for CancellationObserver {
        fn on_start(&self, _event: &InvocationStart<'_>) {
            self.started.notify_one();
        }

        fn on_finish(&self, event: &InvocationFinish<'_>) {
            self.outcomes
                .lock()
                .unwrap()
                .push((event.outcome, event.phase));
        }
    }

    #[tokio::test]
    async fn aborting_an_invocation_reports_cancellation_once() {
        let observer = CancellationObserver::default();
        let runtime = ClientRuntime::builder()
            .observer(observer.clone())
            .middleware(PendingMiddleware)
            .build()
            .unwrap();
        let client = runtime
            .__client_builder(service())
            .direct("http://127.0.0.1:1")
            .connect()
            .await
            .unwrap();
        let task =
            tokio::spawn(async move { client.__invoke(MethodId::__new(0), Vec::new()).await });
        observer.started.notified().await;
        tokio::task::yield_now().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(
            *observer.outcomes.lock().unwrap(),
            [(InvocationOutcome::Cancelled, InvocationPhase::Middleware)]
        );
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn dropping_the_last_runtime_owner_requests_cleanup() {
        let cleanups = Arc::new(AtomicUsize::new(0));
        let runtime = ClientRuntime::builder()
            .registry(CountingRegister {
                subscriptions: Arc::new(AtomicUsize::new(0)),
                cleanups: cleanups.clone(),
            })
            .build()
            .unwrap();
        let client = runtime
            .__client_builder(service())
            .discover()
            .connect()
            .await
            .unwrap();
        drop(runtime);
        drop(client);
        tokio::time::timeout(Duration::from_secs(1), async {
            while cleanups.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
