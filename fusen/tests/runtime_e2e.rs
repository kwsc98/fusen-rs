//! Real-socket coverage for HTTP/1.1, h2c, and bounded server draining.

use fusen_rs::{
    ClientConfig, ClientRuntime, Context, Error, ErrorCategory, ErrorOrigin, Interceptor,
    InterceptorFuture, Next, Response, RetryConfig, Server, ServerConfig, ServerErrorKind,
    ServerState,
    contract::{EndpointCapabilities, HttpVersionPolicy, HttpVersionSet},
    interface,
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
    ) -> Result<Response<String>, Error>;

    #[fusen_rs::method(method = "POST", path = "/users")]
    async fn create(&self, name: String, audit: bool) -> Result<Response<String>, Error>;

    #[fusen_rs::method(method = "GET", path = "/tags")]
    async fn tags(
        &self,
        #[param(query, repeated)] tags: Vec<String>,
    ) -> Result<Response<Vec<String>>, Error>;
}

struct WireServiceImpl;

impl WireService for WireServiceImpl {
    async fn lookup(&self, id: String, expanded: Option<bool>) -> Result<Response<String>, Error> {
        Ok(Response::new(format!(
            "{}:{}",
            id,
            expanded.unwrap_or(false)
        )))
    }

    async fn create(&self, name: String, audit: bool) -> Result<Response<String>, Error> {
        Ok(Response::new(format!("{name}:{audit}")))
    }

    async fn tags(&self, tags: Vec<String>) -> Result<Response<Vec<String>>, Error> {
        Ok(Response::new(tags))
    }
}

#[interface(name = "blocking-e2e")]
trait BlockingService {
    #[fusen_rs::method(method = "PUT", path = "/blocking/wait")]
    async fn wait(&self, #[param(body)] value: String) -> Result<Response<String>, Error>;
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
    async fn wait(&self, value: String) -> Result<Response<String>, Error> {
        let _probe = self.dropped.as_ref().map(|flag| DropProbe(flag.clone()));
        self.entered.wait().await;
        let _permit = self
            .release
            .acquire()
            .await
            .expect("test release semaphore remains open");
        Ok(Response::new(value))
    }
}

#[interface(name = "panic-e2e")]
trait PanicService {
    #[fusen_rs::method(method = "PUT", path = "/panic/execute")]
    async fn execute(&self, #[param(body)] should_panic: bool) -> Result<Response<String>, Error>;
}

struct PanicServiceImpl;

impl PanicService for PanicServiceImpl {
    async fn execute(&self, should_panic: bool) -> Result<Response<String>, Error> {
        assert!(!should_panic, "private panic payload");
        Ok(Response::new("healthy".to_owned()))
    }
}

#[interface(name = "logical-interceptor-e2e")]
trait LogicalInterceptorService {
    #[fusen_rs::method(method = "GET", path = "/logical-interceptor")]
    async fn execute(&self) -> Result<Response<String>, Error>;
}

struct RetryOnceService {
    attempts: Arc<AtomicUsize>,
}

impl LogicalInterceptorService for RetryOnceService {
    async fn execute(&self) -> Result<Response<String>, Error> {
        match self.attempts.fetch_add(1, Ordering::AcqRel) {
            0 => Err(Error::local(
                ErrorCategory::Unavailable,
                "retry_once",
                "retry this safe request once",
            )
            .unwrap()),
            1 => Ok(Response::new("complete".to_owned())),
            attempt => panic!("unexpected physical attempt {}", attempt + 1),
        }
    }
}

struct InvocationCounter(Arc<AtomicUsize>);

impl Interceptor for InvocationCounter {
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        assert_eq!(
            context.attempt(),
            None,
            "logical interceptor is not attempt-scoped"
        );
        self.0.fetch_add(1, Ordering::AcqRel);
        Box::pin(async move { next.run(context).await })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_h2c_and_http1_slices_round_trip() {
    let server = Server::builder("127.0.0.1:0")
        .interface(WireServiceServer::new(WireServiceImpl))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let endpoint = format!("http://{}", server.local_addr());
    let runtime = ClientRuntime::builder().build().unwrap();
    let http1 = WireServiceClient::builder(&runtime)
        .direct(&endpoint)
        .http_version_policy(HttpVersionPolicy::Http1)
        .connect()
        .await
        .unwrap();
    let h2c = WireServiceClient::builder(&runtime)
        .direct(&endpoint)
        .http_version_policy(HttpVersionPolicy::H2c)
        .direct_capabilities(
            EndpointCapabilities::new(HttpVersionSet::ALL, [Default::default()], true).unwrap(),
        )
        .connect()
        .await
        .unwrap();

    assert_eq!(
        http1
            .lookup("http1".into(), Some(true))
            .await
            .unwrap()
            .into_body(),
        "http1:true"
    );
    assert_eq!(
        http1
            .lookup("0".into(), Some(false))
            .await
            .unwrap()
            .into_body(),
        "0:false"
    );
    assert_eq!(
        h2c.lookup("h2c value".into(), Some(false))
            .await
            .unwrap()
            .into_body(),
        "h2c value:false"
    );
    assert_eq!(
        h2c.lookup("missing".into(), None)
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
        assert_eq!(h2c.tags(tags.clone()).await.unwrap().into_body(), tags);
    }
    assert_eq!(
        http1
            .create("http1-created".into(), true)
            .await
            .unwrap()
            .into_body(),
        "http1-created:true"
    );
    assert_eq!(
        h2c.create("created".into(), false)
            .await
            .unwrap()
            .into_body(),
        "created:false"
    );

    drop((http1, h2c));
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_interceptor_runs_once_around_two_physical_attempts() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let server = Server::builder("127.0.0.1:0")
        .interface(LogicalInterceptorServiceServer::new(RetryOnceService {
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
        .interceptor(InvocationCounter(global_calls.clone()))
        .build()
        .unwrap();
    let client = LogicalInterceptorServiceClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .interceptor(InvocationCounter(local_calls.clone()))
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
    let failed = failed.expect_err("panic must become a sanitized invocation error");
    assert_eq!(failed.category(), ErrorCategory::Internal);
    assert_eq!(failed.origin(), ErrorOrigin::Remote);
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
