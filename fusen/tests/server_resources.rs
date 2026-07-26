//! Real-socket coverage for server admission and request-body resource ordering.

use fusen_register::{
    RegistrationHandle, Registry, SubscriptionHandle,
    error::{RegistryError, RegistryErrorKind, RegistryOperation},
    prepare_registration,
};
use fusen_rs::{
    ClientAdmissionConfig, ClientConfig, ClientRuntime, HttpServerConfig, Middleware, Next,
    ProblemDetails, RpcCategory, RpcContext, RpcError, RpcOrigin, RpcResult, Server, ServerConfig,
    ServerRequestConfig, ServerState, WireProtocol,
    contract::{ProtocolSet, ServiceRegistration, ServiceSelector},
    service,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{Notify, Semaphore, oneshot},
};

const SERVER_ADMISSION_LIMIT: usize = 1024;

#[service(name = "server-resource-e2e")]
trait ResourceService {
    #[fusen_rs::method(
        idempotency = "none",
        spring(method = "POST", path = "/resources/echo", body = "value")
    )]
    async fn echo(&self, value: String) -> Result<String, RpcError>;

    #[fusen_rs::method(
        idempotency = "none",
        spring(method = "POST", path = "/resources/panic", body = "value")
    )]
    async fn panic_after_decode(&self, value: String) -> Result<String, RpcError>;

    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/resources/hold/{value}")
    )]
    async fn hold(&self, value: String) -> Result<String, RpcError>;
}

struct ResourceServiceImpl {
    saturation: Option<Arc<Saturation>>,
}

impl ResourceService for ResourceServiceImpl {
    async fn echo(&self, value: String) -> Result<String, RpcError> {
        Ok(value)
    }

    async fn panic_after_decode(&self, value: String) -> Result<String, RpcError> {
        panic!("private panic after decoding {value}")
    }

    async fn hold(&self, value: String) -> Result<String, RpcError> {
        if let Some(saturation) = &self.saturation {
            saturation.entered.fetch_add(1, Ordering::AcqRel);
            saturation.changed.notify_waiters();
            let _permit = saturation
                .release
                .clone()
                .acquire_owned()
                .await
                .expect("test release semaphore remains open");
        }
        Ok(value)
    }
}

struct Saturation {
    entered: AtomicUsize,
    changed: Notify,
    release: Arc<Semaphore>,
}

impl Saturation {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: AtomicUsize::new(0),
            changed: Notify::new(),
            release: Arc::new(Semaphore::new(0)),
        })
    }

    async fn wait_until_full(&self) {
        self.wait_until_entered(SERVER_ADMISSION_LIMIT).await;
    }

    async fn wait_until_entered(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let changed = self.changed.notified();
                if self.entered.load(Ordering::Acquire) >= expected {
                    return;
                }
                changed.await;
            }
        })
        .await
        .expect("the expected admitted requests must enter the service");
    }
}

struct PanicOnceMiddleware(AtomicUsize);

impl Middleware for PanicOnceMiddleware {
    fn handle<'a>(
        &'a self,
        context: RpcContext,
        next: Next<'a>,
    ) -> impl std::future::Future<Output = RpcResult> + Send + 'a {
        if self.0.fetch_add(1, Ordering::AcqRel) == 0 {
            panic!("private synchronous middleware panic");
        }
        async move { next.run(context).await }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_rejections_return_without_receiving_the_declared_body() {
    let server = start_resource_server(body_limited_config(Duration::from_secs(5))).await;
    let addr = server.local_addr();

    let unknown = exchange(
        addr,
        request_head("POST", "/missing", 16, "unknown-route", None),
        &[],
    )
    .await;
    assert_problem(&unknown, 404, "route_not_found");

    let oversized = exchange(
        addr,
        request_head(
            "POST",
            "/resources/echo",
            17,
            "oversized-content-length",
            None,
        ),
        &[],
    )
    .await;
    assert_problem(&oversized, 413, "payload_too_large");

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn not_ready_returns_without_receiving_the_declared_body() {
    let (endpoint_sender, endpoint_receiver) = oneshot::channel();
    let activation_gate = Arc::new(Semaphore::new(0));
    let registry = StartupGateRegistry {
        endpoint_sender: Mutex::new(Some(endpoint_sender)),
        activation_gate: activation_gate.clone(),
    };
    let config = ServerConfig::builder()
        .protocols(ProtocolSet::SPRING_CLOUD_V1)
        .request(
            ServerRequestConfig::default()
                .body_limits(16, 1024)
                .inflight_body_budgets(16, 1024),
        )
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .registry("startup-gate", registry)
        .service(ResourceServiceServer::new(ResourceServiceImpl {
            saturation: None,
        }))
        .build()
        .unwrap();
    let starting = tokio::spawn(server.start());
    let addr = tokio::time::timeout(Duration::from_secs(2), endpoint_receiver)
        .await
        .expect("registration preparation must expose the once-bound endpoint")
        .expect("startup gate must publish its endpoint");

    let response = exchange(
        addr,
        request_head("POST", "/resources/echo", 16, "not-ready-request", None),
        &[],
    )
    .await;
    assert_problem(&response, 503, "not_ready");
    assert!(!problem(&response).retryable);

    activation_gate.add_permits(1);
    let running = starting.await.unwrap().unwrap();
    running.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_1025th_request_is_rejected_while_1024_are_in_flight() {
    let saturation = Saturation::new();
    let server_config = ServerConfig::builder()
        .http(HttpServerConfig::default().http2(2048, None, Duration::from_secs(10)))
        .request(ServerRequestConfig::default().max_concurrent_requests(SERVER_ADMISSION_LIMIT))
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(server_config)
        .service(ResourceServiceServer::new(ResourceServiceImpl {
            saturation: Some(saturation.clone()),
        }))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let client_config = ClientConfig::builder()
        .request_timeout(Duration::from_secs(30))
        .admission(
            ClientAdmissionConfig::default()
                .max_in_flight(2048)
                .max_in_flight_per_endpoint(2048),
        )
        .build()
        .unwrap();
    let runtime = ClientRuntime::builder()
        .config(client_config)
        .build()
        .unwrap();
    let client = ResourceServiceClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .connect()
        .await
        .unwrap();

    let calls = (0..SERVER_ADMISSION_LIMIT)
        .map(|index| {
            let client = client.clone();
            tokio::spawn(async move { client.hold(index.to_string()).await })
        })
        .collect::<Vec<_>>();
    saturation.wait_until_full().await;

    let rejected = tokio::time::timeout(Duration::from_secs(2), client.hold("overflow".into()))
        .await
        .expect("fail-fast admission must not wait for an existing request")
        .expect_err("the 1025th request must be rejected");
    assert_eq!(rejected.category(), RpcCategory::ResourceExhausted);
    assert_eq!(rejected.origin(), RpcOrigin::Remote);
    assert_eq!(rejected.code().as_str(), "overloaded");
    assert_eq!(rejected.attempts(), 1);

    saturation.release.add_permits(SERVER_ADMISSION_LIMIT);
    for call in calls {
        call.await.unwrap().unwrap();
    }
    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_byte_budget_is_restored_after_body_cancellation() {
    let server = start_resource_server(body_limited_config(Duration::from_secs(5))).await;
    let addr = server.local_addr();
    let mut cancelled = TcpStream::connect(addr).await.unwrap();
    cancelled
        .write_all(
            request_head_expecting_continue("POST", "/resources/echo", 16, "cancelled-body", None)
                .as_bytes(),
        )
        .await
        .unwrap();
    wait_for_continue(&mut cancelled).await;
    cancelled.write_all(b"\"").await.unwrap();
    assert_request_budget_is_held(addr).await;

    cancelled.shutdown().await.unwrap();
    let mut discarded = Vec::new();
    let _terminal_read = tokio::time::timeout(
        Duration::from_secs(2),
        cancelled.read_to_end(&mut discarded),
    )
    .await
    .expect("the cancelled HTTP/1 request must terminate");

    assert_echo_succeeds(addr, "after-cancel").await;
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_byte_budget_is_restored_after_body_timeout() {
    let server = start_resource_server(body_limited_config(Duration::from_secs(5))).await;
    let addr = server.local_addr();
    let mut timed_out = TcpStream::connect(addr).await.unwrap();
    timed_out
        .write_all(
            request_head_expecting_continue(
                "POST",
                "/resources/echo",
                16,
                "timed-out-body",
                Some(1000),
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    wait_for_continue(&mut timed_out).await;
    timed_out.write_all(b"\"").await.unwrap();
    assert_request_budget_is_held(addr).await;

    let response = read_response(&mut timed_out).await;
    assert_problem(&response, 504, "deadline_exceeded");
    assert_echo_succeeds(addr, "after-timeout").await;

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_byte_budget_is_restored_after_handler_panic() {
    let server = start_resource_server(body_limited_config(Duration::from_secs(5))).await;
    let addr = server.local_addr();
    let response = exchange(
        addr,
        request_head(
            "POST",
            "/resources/panic",
            b"\"panic\"".len(),
            "panic-after-decode",
            None,
        ),
        b"\"panic\"",
    )
    .await;
    assert_problem(&response, 500, "service_panic");

    assert_echo_succeeds(addr, "after-panic").await;
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_drains_an_inflight_http1_response() {
    let saturation = Saturation::new();
    let config = ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .service(ResourceServiceServer::new(ResourceServiceImpl {
            saturation: Some(saturation.clone()),
        }))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let address = server.local_addr();
    let call = tokio::spawn(async move {
        exchange(
            address,
            request_head("GET", "/resources/hold/http1", 0, "http1-drain", None),
            &[],
        )
        .await
    });
    saturation.wait_until_entered(1).await;

    let handle = server.handle();
    let cancelled_waiter = tokio::spawn(async move { handle.shutdown().await });
    wait_for_state(&server, ServerState::Draining).await;
    cancelled_waiter.abort();
    assert!(cancelled_waiter.await.unwrap_err().is_cancelled());
    let handle = server.handle();
    let shared_terminal = tokio::spawn(async move { handle.shutdown().await });
    saturation.release.add_permits(1);

    let response = call.await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(
        serde_json::from_slice::<String>(&response.body).unwrap(),
        "http1"
    );
    shared_terminal.await.unwrap().unwrap();
    server.wait().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn synchronous_middleware_panic_does_not_poison_an_http1_connection() {
    let config = ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .middleware(PanicOnceMiddleware(AtomicUsize::new(0)))
        .service(ResourceServiceServer::new(ResourceServiceImpl {
            saturation: None,
        }))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let mut stream = TcpStream::connect(server.local_addr()).await.unwrap();

    let first_body = serde_json::to_vec("first").unwrap();
    stream
        .write_all(
            request_head_with_connection(
                "POST",
                "/resources/echo",
                first_body.len(),
                "sync-panic",
                None,
                "keep-alive",
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream.write_all(&first_body).await.unwrap();
    let failed = read_response(&mut stream).await;
    assert_problem(&failed, 500, "middleware_panic");
    assert!(!String::from_utf8_lossy(&failed.body).contains("private synchronous"));

    let second_body = serde_json::to_vec("healthy").unwrap();
    stream
        .write_all(
            request_head_with_connection(
                "POST",
                "/resources/echo",
                second_body.len(),
                "after-sync-panic",
                None,
                "close",
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream.write_all(&second_body).await.unwrap();
    let healthy = read_response(&mut stream).await;
    assert_eq!(healthy.status, 200);
    assert_eq!(
        serde_json::from_slice::<String>(&healthy.body).unwrap(),
        "healthy"
    );

    server.shutdown().await.unwrap();
}

struct StartupGateRegistry {
    endpoint_sender: Mutex<Option<oneshot::Sender<SocketAddr>>>,
    activation_gate: Arc<Semaphore>,
}

impl Registry for StartupGateRegistry {
    fn prepare_registration(
        &self,
        registration: Arc<ServiceRegistration>,
        _protocol: WireProtocol,
    ) -> Result<RegistrationHandle, RegistryError> {
        let endpoint = registration.endpoint().as_url();
        let host = endpoint
            .host_str()
            .expect("test server endpoint contains a host")
            .parse::<IpAddr>()
            .expect("test server binds an IP literal");
        let addr = SocketAddr::new(
            host,
            endpoint
                .port_or_known_default()
                .expect("test server endpoint contains a port"),
        );
        if let Some(sender) = self.endpoint_sender.lock().unwrap().take() {
            let _ = sender.send(addr);
        }
        let activation_gate = self.activation_gate.clone();
        Ok(prepare_registration(
            async move {
                let _permit = activation_gate.acquire_owned().await.map_err(|_| {
                    RegistryError::message(
                        RegistryOperation::ActivateRegistration,
                        RegistryErrorKind::Unavailable,
                        "test activation gate closed",
                    )
                })?;
                Ok(())
            },
            || async { Ok(()) },
        ))
    }

    fn prepare_subscription(
        &self,
        _selector: ServiceSelector,
        _protocol: WireProtocol,
    ) -> Result<SubscriptionHandle, RegistryError> {
        Err(RegistryError::message(
            RegistryOperation::PrepareSubscription,
            RegistryErrorKind::InvalidResource,
            "test registry does not support subscriptions",
        ))
    }
}

struct RawResponse {
    status: u16,
    body: Vec<u8>,
}

async fn start_resource_server(config: ServerConfig) -> fusen_rs::RunningServer {
    Server::builder("127.0.0.1:0")
        .config(config)
        .service(ResourceServiceServer::new(ResourceServiceImpl {
            saturation: None,
        }))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap()
}

fn body_limited_config(timeout: Duration) -> ServerConfig {
    ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .request(
            ServerRequestConfig::default()
                .timeout(timeout)
                .body_limits(16, 1024)
                .inflight_body_budgets(16, 1024),
        )
        .build()
        .unwrap()
}

fn request_head(
    method: &str,
    path: &str,
    content_length: usize,
    request_id: &str,
    timeout_ms: Option<u64>,
) -> String {
    request_head_with_options(
        method,
        path,
        content_length,
        request_id,
        timeout_ms,
        "close",
        false,
    )
}

fn request_head_expecting_continue(
    method: &str,
    path: &str,
    content_length: usize,
    request_id: &str,
    timeout_ms: Option<u64>,
) -> String {
    request_head_with_options(
        method,
        path,
        content_length,
        request_id,
        timeout_ms,
        "close",
        true,
    )
}

fn request_head_with_connection(
    method: &str,
    path: &str,
    content_length: usize,
    request_id: &str,
    timeout_ms: Option<u64>,
    connection: &str,
) -> String {
    request_head_with_options(
        method,
        path,
        content_length,
        request_id,
        timeout_ms,
        connection,
        false,
    )
}

fn request_head_with_options(
    method: &str,
    path: &str,
    content_length: usize,
    request_id: &str,
    timeout_ms: Option<u64>,
    connection: &str,
    expect_continue: bool,
) -> String {
    let timeout = timeout_ms
        .map(|timeout| format!("x-fusen-timeout-ms: {timeout}\r\n"))
        .unwrap_or_default();
    let expect = if expect_continue {
        "Expect: 100-continue\r\n"
    } else {
        ""
    };
    format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: localhost\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {content_length}\r\n\
         x-request-id: {request_id}\r\n\
         {timeout}\
         {expect}\
         Connection: {connection}\r\n\
         \r\n"
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

async fn exchange(addr: SocketAddr, head: String, body: &[u8]) -> RawResponse {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    read_response(&mut stream).await
}

async fn read_response(stream: &mut TcpStream) -> RawResponse {
    tokio::time::timeout(Duration::from_secs(2), read_response_inner(stream))
        .await
        .expect("server must answer without waiting for unsent request bytes")
}

async fn read_response_inner(stream: &mut TcpStream) -> RawResponse {
    let mut received = Vec::new();
    let mut buffer = [0u8; 2048];
    loop {
        let read = stream.read(&mut buffer).await.unwrap();
        assert!(read > 0, "HTTP response ended before its headers completed");
        received.extend_from_slice(&buffer[..read]);
        let Some(head_end) = find_bytes(&received, b"\r\n\r\n") else {
            continue;
        };
        let body_start = head_end + 4;
        let head = std::str::from_utf8(&received[..head_end]).unwrap();
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_ascii_whitespace().nth(1))
            .and_then(|status| status.parse::<u16>().ok())
            .expect("server returned a valid HTTP status line");
        let content_length = head
            .lines()
            .skip(1)
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .expect("bounded server responses carry Content-Length");
        while received.len() < body_start + content_length {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0, "HTTP response ended before its body completed");
            received.extend_from_slice(&buffer[..read]);
        }
        return RawResponse {
            status,
            body: received[body_start..body_start + content_length].to_vec(),
        };
    }
}

async fn wait_for_continue(stream: &mut TcpStream) {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut received = Vec::new();
        let mut buffer = [0u8; 256];
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0, "HTTP connection ended before 100 Continue");
            received.extend_from_slice(&buffer[..read]);
            if find_bytes(&received, b"\r\n\r\n").is_some() {
                assert_eq!(received, b"HTTP/1.1 100 Continue\r\n\r\n");
                return;
            }
        }
    })
    .await
    .expect("polling the request body must emit 100 Continue");
}

async fn assert_request_budget_is_held(addr: SocketAddr) {
    let body = b"\"probe\"";
    let response = exchange(
        addr,
        request_head("POST", "/resources/echo", body.len(), "budget-probe", None),
        body,
    )
    .await;
    assert_problem(&response, 429, "body_byte_budget_exhausted");
}

async fn assert_echo_succeeds(addr: SocketAddr, value: &str) {
    let body = serde_json::to_vec(value).unwrap();
    assert!(body.len() <= 16);
    let response = exchange(
        addr,
        request_head(
            "POST",
            "/resources/echo",
            body.len(),
            "budget-recovered",
            None,
        ),
        &body,
    )
    .await;
    assert_eq!(response.status, 200);
    assert_eq!(
        serde_json::from_slice::<String>(&response.body).unwrap(),
        value
    );
}

fn assert_problem(response: &RawResponse, status: u16, code: &str) {
    assert_eq!(response.status, status);
    let problem = problem(response);
    assert_eq!(problem.status, status);
    assert_eq!(problem.code.as_str(), code);
}

fn problem(response: &RawResponse) -> ProblemDetails {
    serde_json::from_slice(&response.body).expect("error response is Problem Details JSON")
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
