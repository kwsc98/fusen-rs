//! Real-socket coverage for both wire protocols and bounded server draining.

use fusen_rs::{
    ClientRuntime, RpcCategory, RpcError, RpcOrigin, Server, ServerConfig, ServerErrorKind,
    ServerState, WireProtocol, contract::ProtocolSet, service,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Barrier, Semaphore};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CreateRequest {
    name: String,
}

#[service(name = "wire-e2e")]
trait WireService {
    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/users/{id}", query = ["expanded"])
    )]
    async fn lookup(&self, id: String, expanded: Option<bool>) -> Result<String, RpcError>;

    #[fusen_rs::method(
        idempotency = "none",
        spring(method = "POST", path = "/users", body = "request")
    )]
    async fn create(&self, request: CreateRequest) -> Result<String, RpcError>;
}

struct WireServiceImpl;

impl WireService for WireServiceImpl {
    async fn lookup(&self, id: String, expanded: Option<bool>) -> Result<String, RpcError> {
        Ok(format!("{id}:{}", expanded.unwrap_or(false)))
    }

    async fn create(&self, request: CreateRequest) -> Result<String, RpcError> {
        Ok(request.name)
    }
}

#[service(name = "blocking-e2e")]
trait BlockingService {
    #[fusen_rs::method(idempotency = "safe")]
    async fn wait(&self, value: String) -> Result<String, RpcError>;
}

struct BlockingServiceImpl {
    entered: Arc<Barrier>,
    release: Arc<Semaphore>,
}

impl BlockingService for BlockingServiceImpl {
    async fn wait(&self, value: String) -> Result<String, RpcError> {
        self.entered.wait().await;
        let _permit = self
            .release
            .acquire()
            .await
            .expect("test release semaphore remains open");
        Ok(value)
    }
}

#[service(name = "panic-e2e")]
trait PanicService {
    #[fusen_rs::method(idempotency = "safe")]
    async fn execute(&self, should_panic: bool) -> Result<String, RpcError>;
}

struct PanicServiceImpl;

impl PanicService for PanicServiceImpl {
    async fn execute(&self, should_panic: bool) -> Result<String, RpcError> {
        assert!(!should_panic, "private panic payload");
        Ok("healthy".to_owned())
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
        .service(WireServiceServer::new(WireServiceImpl))
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
        fusen.lookup("fusen".into(), Some(true)).await.unwrap(),
        "fusen:true"
    );
    assert_eq!(
        spring
            .lookup("spring cloud".into(), Some(false))
            .await
            .unwrap(),
        "spring cloud:false"
    );
    assert_eq!(
        spring
            .create(CreateRequest {
                name: "created".into()
            })
            .await
            .unwrap(),
        "created"
    );

    drop((fusen, spring));
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_drains_an_inflight_h2_stream() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Semaphore::new(0));
    let server = Server::builder("127.0.0.1:0")
        .service(BlockingServiceServer::new(BlockingServiceImpl {
            entered: entered.clone(),
            release: release.clone(),
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

    assert_eq!(call.await.unwrap().unwrap(), "complete");
    shutdown.await.unwrap().unwrap();
    server.wait().await.unwrap();
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_aborts_a_permanently_pending_stream_at_deadline() {
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Semaphore::new(0));
    let config = ServerConfig::builder()
        .graceful_shutdown_timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .service(BlockingServiceServer::new(BlockingServiceImpl {
            entered: entered.clone(),
            release,
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
    let call = tokio::spawn(async move { client.wait("never".into()).await });
    entered.wait().await;

    let shutdown = tokio::time::timeout(Duration::from_secs(1), server.shutdown())
        .await
        .expect("server shutdown must remain bounded")
        .expect_err("pending stream must exhaust the graceful deadline");
    assert_eq!(shutdown.kind(), ServerErrorKind::Timeout);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), call)
            .await
            .expect("aborted call must terminate")
            .unwrap()
            .is_err()
    );
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_h2_stream_does_not_poison_other_streams() {
    let server = Server::builder("127.0.0.1:0")
        .service(PanicServiceServer::new(PanicServiceImpl))
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

    let (failed, healthy) = tokio::join!(client.execute(true), client.execute(false));
    let failed = failed.expect_err("panic must become a sanitized RPC error");
    assert_eq!(failed.category(), RpcCategory::Internal);
    assert_eq!(failed.origin(), RpcOrigin::Remote);
    assert!(!failed.message().contains("private panic payload"));
    assert_eq!(healthy.unwrap(), "healthy");
    assert_eq!(client.execute(false).await.unwrap(), "healthy");

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
