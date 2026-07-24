use crate::{
    ClientConfig, ClientRuntime,
    contract::WireProtocol,
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
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
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
