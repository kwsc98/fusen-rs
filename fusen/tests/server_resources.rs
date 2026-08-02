//! Real-socket coverage for server admission and request-body resource ordering.

use bytes::Bytes;
use fusen_register::{
    RegistrationHandle, RegistrationRequest, Registry, SubscriptionHandle, SubscriptionRequest,
    error::{RegistryError, RegistryErrorKind, RegistryOperation},
    provider,
};
use fusen_rs::{
    ClientAdmissionConfig, ClientConfig, ClientRuntime, Context, Error, ErrorCategory, ErrorOrigin,
    HttpServerConfig, Interceptor, InterceptorFuture, Next, Response, RetryConfig, Server,
    ServerConfig, ServerRequestConfig, ServerState, interface,
};
use futures_util::{StreamExt as _, stream};
use http::{Method, Request, StatusCode, Version};
use http_body_util::{BodyExt, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::Frame;
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde::Deserialize;
use std::{
    convert::Infallible,
    io,
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

#[interface(name = "server-resource-e2e")]
trait ResourceService {
    #[fusen_rs::method(method = "POST", path = "/resources/echo")]
    async fn echo(&self, #[param(body)] value: String) -> Result<Response<String>, Error>;

    #[fusen_rs::method(method = "POST", path = "/resources/panic")]
    async fn panic_after_decode(
        &self,
        #[param(body)] value: String,
    ) -> Result<Response<String>, Error>;

    #[fusen_rs::method(method = "GET", path = "/resources/hold/{value}")]
    async fn hold(&self, value: String) -> Result<Response<String>, Error>;

    #[fusen_rs::method(method = "HEAD", path = "/resources/health")]
    async fn health(&self) -> Result<Response<()>, Error>;

    #[fusen_rs::method(method = "OPTIONS", path = "/resources/options")]
    async fn options(&self) -> Result<Response<String>, Error>;
}

struct ResourceServiceImpl {
    saturation: Option<Arc<Saturation>>,
}

impl ResourceService for ResourceServiceImpl {
    async fn echo(&self, value: String) -> Result<Response<String>, Error> {
        Ok(Response::new(value))
    }

    async fn panic_after_decode(&self, value: String) -> Result<Response<String>, Error> {
        panic!("private panic after decoding {value}")
    }

    async fn hold(&self, value: String) -> Result<Response<String>, Error> {
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
        Ok(Response::new(value))
    }

    async fn health(&self) -> Result<Response<()>, Error> {
        Ok(Response::new(()))
    }

    async fn options(&self) -> Result<Response<String>, Error> {
        Ok(Response::new("options".to_owned()))
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
        let completed = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let changed = self.changed.notified();
                if self.entered.load(Ordering::Acquire) >= expected {
                    return;
                }
                changed.await;
            }
        })
        .await;
        assert!(
            completed.is_ok(),
            "expected {expected} admitted requests to enter the handler, observed {}",
            self.entered.load(Ordering::Acquire)
        );
    }
}

struct PanicOnceInterceptor(AtomicUsize);

impl Interceptor for PanicOnceInterceptor {
    fn intercept<'a>(&'a self, context: Context, next: Next<'a>) -> InterceptorFuture<'a> {
        if self.0.fetch_add(1, Ordering::AcqRel) == 0 {
            panic!("private synchronous interceptor panic");
        }
        Box::pin(async move { next.run(context).await })
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
async fn no_body_routes_reject_chunked_http1_before_sending_continue() {
    let server = start_resource_server(ServerConfig::default()).await;
    let addr = server.local_addr();

    for (method, path) in [
        ("GET", "/resources/hold/chunked"),
        ("HEAD", "/resources/health"),
        ("OPTIONS", "/resources/options"),
    ] {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "{}4\r\nnull\r\n0\r\n\r\n",
            chunked_request_head(method, path, "chunked-no-body")
        );
        stream.write_all(request.as_bytes()).await.unwrap();

        if method == "HEAD" {
            assert_eq!(read_response_status(&mut stream).await, 400);
        } else {
            let response = read_response(&mut stream).await;
            assert_problem(&response, 400, "unexpected_body");
        }
    }

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_body_routes_reject_http2_data_without_content_length() {
    let server = start_resource_server(ServerConfig::default()).await;
    let addr = server.local_addr();

    for (method, path) in [
        (Method::GET, "/resources/hold/h2-data"),
        (Method::HEAD, "/resources/health"),
        (Method::OPTIONS, "/resources/options"),
    ] {
        let body = StreamBody::new(
            stream::once(async { Ok::<_, Infallible>(Frame::data(Bytes::from_static(b"null"))) })
                .chain(stream::pending()),
        )
        .boxed_unsync();
        let (status, response_body) = h2_exchange(addr, method.clone(), path, body).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        if method == Method::HEAD {
            assert!(response_body.is_empty());
        } else {
            let response = RawResponse {
                status: status.as_u16(),
                request_id: None,
                body: response_body.to_vec(),
            };
            assert_problem(&response, 400, "unexpected_body");
        }
    }

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_length_empty_http2_bodies_remain_valid_for_no_body_routes() {
    let server = start_resource_server(ServerConfig::default()).await;
    let addr = server.local_addr();

    for (method, path, expected) in [
        (Method::GET, "/resources/hold/h2-empty", Some("h2-empty")),
        (Method::HEAD, "/resources/health", None),
        (Method::OPTIONS, "/resources/options", Some("options")),
    ] {
        let body =
            StreamBody::new(stream::empty::<Result<Frame<Bytes>, Infallible>>()).boxed_unsync();
        let (status, response_body) = h2_exchange(addr, method, path, body).await;

        assert_eq!(status, StatusCode::OK);
        match expected {
            Some(expected) => {
                assert_eq!(
                    serde_json::from_slice::<String>(&response_body).unwrap(),
                    expected
                )
            }
            None => assert!(response_body.is_empty()),
        }
    }

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_http2_body_validation_obeys_deadline_and_releases_resources() {
    let config = ServerConfig::builder()
        .request(
            ServerRequestConfig::builder()
                .timeout(Duration::from_millis(250))
                .max_concurrent_requests(1)
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let server = start_resource_server(config).await;
    let addr = server.local_addr();
    let body =
        StreamBody::new(stream::pending::<Result<Frame<Bytes>, Infallible>>()).boxed_unsync();

    let (status, response_body) =
        h2_exchange(addr, Method::GET, "/resources/hold/pending", body).await;
    let response = RawResponse {
        status: status.as_u16(),
        request_id: None,
        body: response_body.to_vec(),
    };
    assert_problem(&response, 504, "deadline_exceeded");

    let recovered = exchange(
        addr,
        request_head("GET", "/resources/hold/recovered", 0, "recovered", None),
        &[],
    )
    .await;
    assert_eq!(recovered.status, 200);

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn early_route_rejection_echoes_a_valid_request_id() {
    let server = start_resource_server(ServerConfig::default()).await;
    let response = exchange(
        server.local_addr(),
        request_head("POST", "/disabled/interface", 0, "early-rejection-id", None),
        &[],
    )
    .await;

    assert_problem(&response, 404, "route_not_found");
    assert_eq!(response.request_id.as_deref(), Some("early-rejection-id"));
    assert_eq!(problem(&response).request_id, "early-rejection-id");

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
        .request(
            ServerRequestConfig::builder()
                .max_request_body_bytes(16)
                .max_response_body_bytes(1024)
                .max_inflight_request_body_bytes(16)
                .max_inflight_response_body_bytes(1024)
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .registry("startup-gate", registry)
        .interface(ResourceServiceServer::new(ResourceServiceImpl {
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
        .http(
            HttpServerConfig::builder()
                .http2_max_concurrent_streams(2048)
                .http2_keep_alive_interval(None)
                .http2_keep_alive_timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        )
        .request(
            ServerRequestConfig::builder()
                .max_concurrent_requests(SERVER_ADMISSION_LIMIT)
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(server_config)
        .interface(ResourceServiceServer::new(ResourceServiceImpl {
            saturation: Some(saturation.clone()),
        }))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let client_config = ClientConfig::builder()
        .request_timeout(Duration::from_secs(30))
        .retry(RetryConfig::builder().max_attempts(1).build().unwrap())
        .admission(
            ClientAdmissionConfig::builder()
                .max_in_flight(2048)
                .max_in_flight_per_endpoint(2048)
                .build()
                .unwrap(),
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
    assert_eq!(rejected.category(), ErrorCategory::ResourceExhausted);
    assert_eq!(rejected.origin(), ErrorOrigin::Remote);
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

    match try_read_response(&mut timed_out).await {
        Ok(response) => assert_problem(&response, 504, "deadline_exceeded"),
        Err(error) if cfg!(windows) && error.kind() == io::ErrorKind::ConnectionAborted => {
            // Windows can abort an HTTP/1 connection whose request body remains unread when the
            // timeout response closes it. Budget recovery below remains the portable contract.
        }
        Err(error) => {
            panic!("timed-out request must return a response or close on Windows: {error}")
        }
    }
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
    assert_problem(&response, 500, "handler_panic");

    assert_echo_succeeds(addr, "after-panic").await;
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn graceful_shutdown_drains_an_inflight_http1_response() {
    let saturation = Saturation::new();
    let server = Server::builder("127.0.0.1:0")
        .interface(ResourceServiceServer::new(ResourceServiceImpl {
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
async fn synchronous_interceptor_panic_does_not_poison_an_http1_connection() {
    let server = Server::builder("127.0.0.1:0")
        .interceptor(PanicOnceInterceptor(AtomicUsize::new(0)))
        .interface(ResourceServiceServer::new(ResourceServiceImpl {
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
    assert_problem(&failed, 500, "interceptor_panic");
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
        request: RegistrationRequest,
    ) -> Result<RegistrationHandle, RegistryError> {
        let registration = request.registration();
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
        Ok(provider::registration(
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
        _request: SubscriptionRequest,
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
    request_id: Option<String>,
    body: Vec<u8>,
}

async fn start_resource_server(config: ServerConfig) -> fusen_rs::RunningServer {
    Server::builder("127.0.0.1:0")
        .config(config)
        .interface(ResourceServiceServer::new(ResourceServiceImpl {
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
        .request(
            ServerRequestConfig::builder()
                .timeout(timeout)
                .max_request_body_bytes(16)
                .max_response_body_bytes(1024)
                .max_inflight_request_body_bytes(16)
                .max_inflight_response_body_bytes(1024)
                .build()
                .unwrap(),
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

fn chunked_request_head(method: &str, path: &str, request_id: &str) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\n\
         Host: localhost\r\n\
         Content-Type: application/json\r\n\
         Transfer-Encoding: chunked\r\n\
         Expect: 100-continue\r\n\
         x-request-id: {request_id}\r\n\
         Connection: close\r\n\
         \r\n"
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

async fn h2_exchange(
    addr: SocketAddr,
    method: Method,
    path: &str,
    body: UnsyncBoxBody<Bytes, Infallible>,
) -> (StatusCode, Bytes) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http2::Builder::new(TokioExecutor::new())
        .handshake(TokioIo::new(stream))
        .await
        .unwrap();
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let request = Request::builder()
        .method(method)
        .version(Version::HTTP_2)
        .uri(format!("http://{addr}{path}"))
        .header("content-type", "application/json")
        .header("x-request-id", "h2-no-body-test")
        .body(body)
        .unwrap();
    assert!(
        !request.headers().contains_key("content-length"),
        "fixture must exercise an HTTP/2 body without Content-Length"
    );
    let response = tokio::time::timeout(Duration::from_secs(2), sender.send_request(request))
        .await
        .expect("server must answer the HTTP/2 request")
        .unwrap();
    let status = response.status();
    let body = tokio::time::timeout(Duration::from_secs(2), response.into_body().collect())
        .await
        .expect("server HTTP/2 response body must complete")
        .unwrap()
        .to_bytes();
    drop(sender);
    connection.abort();
    let _ = connection.await;
    (status, body)
}

async fn read_response(stream: &mut TcpStream) -> RawResponse {
    try_read_response(stream)
        .await
        .expect("server response must be readable")
}

async fn read_response_status(stream: &mut TcpStream) -> u16 {
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut received = Vec::new();
        let mut buffer = [0u8; 512];
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0, "HTTP response ended before its headers completed");
            received.extend_from_slice(&buffer[..read]);
            let Some(head_end) = find_bytes(&received, b"\r\n\r\n") else {
                continue;
            };
            return std::str::from_utf8(&received[..head_end])
                .unwrap()
                .lines()
                .next()
                .and_then(|line| line.split_ascii_whitespace().nth(1))
                .and_then(|status| status.parse::<u16>().ok())
                .expect("server returned a valid HTTP status line");
        }
    })
    .await
    .expect("server must reject before waiting for a chunked request body")
}

async fn try_read_response(stream: &mut TcpStream) -> io::Result<RawResponse> {
    tokio::time::timeout(Duration::from_secs(2), read_response_inner(stream))
        .await
        .expect("server must answer without waiting for unsent request bytes")
}

async fn read_response_inner(stream: &mut TcpStream) -> io::Result<RawResponse> {
    let mut received = Vec::new();
    let mut buffer = [0u8; 2048];
    loop {
        let read = stream.read(&mut buffer).await?;
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
        let request_id = head.lines().skip(1).find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("x-request-id")
                .then(|| value.trim().to_owned())
        });
        while received.len() < body_start + content_length {
            let read = stream.read(&mut buffer).await?;
            assert!(read > 0, "HTTP response ended before its body completed");
            received.extend_from_slice(&buffer[..read]);
        }
        return Ok(RawResponse {
            status,
            request_id,
            body: received[body_start..body_start + content_length].to_vec(),
        });
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

#[derive(Deserialize)]
struct WireProblem {
    status: u16,
    code: String,
    request_id: String,
    retryable: bool,
}

fn problem(response: &RawResponse) -> WireProblem {
    serde_json::from_slice(&response.body).expect("error response is Problem Details JSON")
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
