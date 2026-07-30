//! Real-socket coverage for both wire protocols and bounded server draining.

use fusen_rs::{
    ClientConfig, ClientRuntime, Middleware, MiddlewareFuture, Next, RetryConfig, RpcCategory,
    RpcContext, RpcError, RpcOrigin, RpcResponse, Server, ServerConfig, ServerErrorKind,
    ServerState, WireProtocol, contract::ProtocolSet, interface,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Barrier, Notify, Semaphore};

#[interface(name = "wire-e2e")]
trait WireService {
    #[fusen_rs::method(method = "GET", path = "/users/{id}")]
    async fn lookup(
        &self,
        id: String,
        #[param(query)] expanded: Option<bool>,
    ) -> Result<RpcResponse<String>, RpcError>;

    #[fusen_rs::method(method = "POST", path = "/users")]
    async fn create(&self, name: String, audit: bool) -> Result<RpcResponse<String>, RpcError>;

    #[fusen_rs::method(method = "GET", path = "/tags")]
    async fn tags(
        &self,
        #[param(query)] tags: Vec<String>,
    ) -> Result<RpcResponse<Vec<String>>, RpcError>;
}

struct WireServiceImpl;

impl WireService for WireServiceImpl {
    async fn lookup(
        &self,
        id: String,
        expanded: Option<bool>,
    ) -> Result<RpcResponse<String>, RpcError> {
        Ok(RpcResponse::new(format!(
            "{}:{}",
            id,
            expanded.unwrap_or(false)
        )))
    }

    async fn create(&self, name: String, audit: bool) -> Result<RpcResponse<String>, RpcError> {
        Ok(RpcResponse::new(format!("{name}:{audit}")))
    }

    async fn tags(&self, tags: Vec<String>) -> Result<RpcResponse<Vec<String>>, RpcError> {
        Ok(RpcResponse::new(tags))
    }
}

#[interface(name = "blocking-e2e")]
trait BlockingService {
    #[fusen_rs::method(method = "PUT", path = "/blocking/wait")]
    async fn wait(&self, #[param(body)] value: String) -> Result<RpcResponse<String>, RpcError>;
}

struct BlockingServiceImpl {
    entered: Arc<Barrier>,
    release: Arc<Semaphore>,
    dropped: Option<Arc<Notify>>,
}

struct DropProbe(Arc<Notify>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.notify_one();
    }
}

impl BlockingService for BlockingServiceImpl {
    async fn wait(&self, value: String) -> Result<RpcResponse<String>, RpcError> {
        let _probe = self.dropped.as_ref().map(|flag| DropProbe(flag.clone()));
        self.entered.wait().await;
        let _permit = self
            .release
            .acquire()
            .await
            .expect("test release semaphore remains open");
        Ok(RpcResponse::new(value))
    }
}

#[interface(name = "panic-e2e")]
trait PanicService {
    #[fusen_rs::method(method = "PUT", path = "/panic/execute")]
    async fn execute(
        &self,
        #[param(body)] should_panic: bool,
    ) -> Result<RpcResponse<String>, RpcError>;
}

struct PanicServiceImpl;

impl PanicService for PanicServiceImpl {
    async fn execute(&self, should_panic: bool) -> Result<RpcResponse<String>, RpcError> {
        assert!(!should_panic, "private panic payload");
        Ok(RpcResponse::new("healthy".to_owned()))
    }
}

#[interface(name = "logical-middleware-e2e")]
trait LogicalMiddlewareService {
    #[fusen_rs::method(method = "GET", path = "/logical-middleware")]
    async fn execute(&self) -> Result<RpcResponse<String>, RpcError>;
}

struct RetryOnceService {
    attempts: Arc<AtomicUsize>,
}

impl LogicalMiddlewareService for RetryOnceService {
    async fn execute(&self) -> Result<RpcResponse<String>, RpcError> {
        match self.attempts.fetch_add(1, Ordering::AcqRel) {
            0 => Err(RpcError::new(
                RpcCategory::Unavailable,
                "retry_once",
                "retry this safe request once",
            )
            .unwrap()),
            1 => Ok(RpcResponse::new("complete".to_owned())),
            attempt => panic!("unexpected physical attempt {}", attempt + 1),
        }
    }
}

struct InvocationCounter(Arc<AtomicUsize>);

impl Middleware for InvocationCounter {
    fn call<'a>(&'a self, context: RpcContext, next: Next<'a>) -> MiddlewareFuture<'a> {
        assert_eq!(
            context.attempt(),
            None,
            "logical middleware is not attempt-scoped"
        );
        self.0.fetch_add(1, Ordering::AcqRel);
        Box::pin(async move { next.run(context).await })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_h2c_and_http1_slices_round_trip() {
    let config = ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .interface(WireServiceServer::new(WireServiceImpl))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let endpoint = format!("http://{}", server.local_addr());
    let runtime = ClientRuntime::builder().build().unwrap();
    let fusen = WireServiceClient::builder(&runtime)
        .direct(&endpoint)
        .protocol(WireProtocol::FusenV1)
        .connect()
        .await
        .unwrap();
    let spring = WireServiceClient::builder(&runtime)
        .direct(&endpoint)
        .protocol(WireProtocol::SpringCloudV1)
        .connect()
        .await
        .unwrap();

    assert_eq!(
        fusen
            .lookup("fusen".into(), Some(true))
            .await
            .unwrap()
            .into_body(),
        "fusen:true"
    );
    assert_eq!(
        fusen
            .lookup("0".into(), Some(false))
            .await
            .unwrap()
            .into_body(),
        "0:false"
    );
    assert_eq!(
        spring
            .lookup("spring cloud".into(), Some(false))
            .await
            .unwrap()
            .into_body(),
        "spring cloud:false"
    );
    assert_eq!(
        spring
            .lookup("missing".into(), None)
            .await
            .unwrap()
            .into_body(),
        "missing:false"
    );
    for tags in [
        Vec::new(),
        vec!["one".to_owned()],
        vec!["one".to_owned(), "two words".to_owned(), "three".to_owned()],
    ] {
        assert_eq!(spring.tags(tags.clone()).await.unwrap().into_body(), tags);
    }
    assert_eq!(
        fusen
            .create("fusen-created".into(), true)
            .await
            .unwrap()
            .into_body(),
        "fusen-created:true"
    );
    assert_eq!(
        spring
            .create("created".into(), false)
            .await
            .unwrap()
            .into_body(),
        "created:false"
    );

    drop((fusen, spring));
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_middleware_runs_once_around_two_physical_attempts() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let server = Server::builder("127.0.0.1:0")
        .config(
            ServerConfig::builder()
                .protocols(ProtocolSet::ALL)
                .build()
                .unwrap(),
        )
        .interface(LogicalMiddlewareServiceServer::new(RetryOnceService {
            attempts: attempts.clone(),
        }))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let global_calls = Arc::new(AtomicUsize::new(0));
    let local_calls = Arc::new(AtomicUsize::new(0));
    let config = ClientConfig::builder()
        .retry(
            RetryConfig::builder()
                .max_attempts(2)
                .backoff_base(Duration::from_nanos(1))
                .backoff_cap(Duration::from_nanos(1))
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let runtime = ClientRuntime::builder()
        .config(config)
        .middleware(InvocationCounter(global_calls.clone()))
        .build()
        .unwrap();
    let client = LogicalMiddlewareServiceClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .protocol(WireProtocol::SpringCloudV1)
        .middleware(InvocationCounter(local_calls.clone()))
        .connect()
        .await
        .unwrap();

    assert_eq!(client.execute().await.unwrap().into_body(), "complete");
    assert_eq!(attempts.load(Ordering::Acquire), 2);
    assert_eq!(global_calls.load(Ordering::Acquire), 1);
    assert_eq!(local_calls.load(Ordering::Acquire), 1);

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_drains_an_inflight_h2_stream() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Semaphore::new(0));
    let server = Server::builder("127.0.0.1:0")
        .interface(BlockingServiceServer::new(BlockingServiceImpl {
            entered: entered.clone(),
            release: release.clone(),
            dropped: None,
        }))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = BlockingServiceClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .connect()
        .await
        .unwrap();

    let call = tokio::spawn(async move { client.wait("complete".into()).await });
    entered.wait().await;
    let handle = server.handle();
    let shutdown = tokio::spawn(async move { handle.shutdown().await });
    wait_for_state(&server, ServerState::Draining).await;
    release.add_permits(1);

    assert_eq!(call.await.unwrap().unwrap().into_body(), "complete");
    shutdown.await.unwrap().unwrap();
    server.wait().await.unwrap();
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_aborts_a_permanently_pending_stream_at_deadline() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Semaphore::new(0));
    let handler_dropped = Arc::new(Notify::new());
    let config = ServerConfig::builder()
        .graceful_shutdown_timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .interface(BlockingServiceServer::new(BlockingServiceImpl {
            entered: entered.clone(),
            release,
            dropped: Some(handler_dropped.clone()),
        }))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let client_config = ClientConfig::builder()
        .retry(RetryConfig::builder().max_attempts(1).build().unwrap())
        .build()
        .unwrap();
    let runtime = ClientRuntime::builder()
        .config(client_config)
        .build()
        .unwrap();
    let client = BlockingServiceClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .connect()
        .await
        .unwrap();
    let mut call = tokio::spawn(async move { client.wait("never".into()).await });
    entered.wait().await;

    let shutdown = tokio::time::timeout(Duration::from_secs(1), server.shutdown())
        .await
        .expect("server shutdown must remain bounded")
        .expect_err("pending stream must exhaust the graceful deadline");
    assert_eq!(shutdown.kind(), ServerErrorKind::Timeout);
    tokio::time::timeout(Duration::from_secs(1), handler_dropped.notified())
        .await
        .expect("pending handler must observe forced shutdown");
    let call_result = match tokio::time::timeout(Duration::from_secs(1), &mut call).await {
        Ok(result) => result,
        Err(error) => {
            call.abort();
            let _ = call.await;
            panic!("aborted call must terminate: {error:?}");
        }
    };
    let call_error = call_result
        .unwrap()
        .expect_err("forced shutdown must fail the pending call");
    assert_eq!(call_error.attempts(), 1);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_h2_stream_does_not_poison_other_streams() {
    let server = Server::builder("127.0.0.1:0")
        .interface(PanicServiceServer::new(PanicServiceImpl))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = PanicServiceClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .connect()
        .await
        .unwrap();

    let (failed, healthy) = tokio::join!(client.execute(true), client.execute(false),);
    let failed = failed.expect_err("panic must become a sanitized RPC error");
    assert_eq!(failed.category(), RpcCategory::Internal);
    assert_eq!(failed.origin(), RpcOrigin::Remote);
    assert!(!failed.message().contains("private panic payload"));
    assert_eq!(healthy.unwrap().into_body(), "healthy");
    assert_eq!(client.execute(false).await.unwrap().into_body(), "healthy");

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

async fn wait_for_state(server: &fusen_rs::RunningServer, expected: ServerState) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while server.state() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("server state transition must complete");
}
