//! Golden contract coverage for the two versioned JSON wire protocols.

use bytes::Bytes;
use fusen_rs::{
    ClientRuntime, ProblemDetails, RpcCategory, RpcError, Server, ServerConfig, WireProtocol,
    contract::ProtocolSet, service,
};
use http::{HeaderMap, Method, Request, Response, StatusCode, Uri, Version, header::CONTENT_TYPE};
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{convert::Infallible, net::SocketAddr};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};

const FUSEN_CONTENT_TYPE: &str = "application/fusen+json;version=1";
const JSON_CONTENT_TYPE: &str = "application/json";
const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CreateUser {
    name: String,
}

#[service(name = "wire-contract", group = "prod", version = "1")]
trait WireContract {
    #[fusen_rs::method(idempotency = "safe", spring(method = "GET", path = "/echo/{name}"))]
    async fn echo(&self, name: String) -> Result<String, RpcError>;

    #[fusen_rs::method(
        idempotency = "none",
        spring(
            method = "POST",
            path = "/users/{id}",
            query = ["expand"],
            body = "request"
        )
    )]
    async fn create(
        &self,
        id: String,
        expand: Option<bool>,
        request: CreateUser,
    ) -> Result<String, RpcError>;

    #[fusen_rs::method(idempotency = "safe", spring(method = "HEAD", path = "/health"))]
    async fn health(&self) -> Result<(), RpcError>;

    #[fusen_rs::method(idempotency = "safe", spring(method = "HEAD", path = "/unhealthy"))]
    async fn unhealthy(&self) -> Result<(), RpcError>;

    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/filters", query = ["enabled"])
    )]
    async fn filter(&self, enabled: Option<bool>) -> Result<Option<bool>, RpcError>;

    #[fusen_rs::method(
        idempotency = "safe",
        spring(method = "GET", path = "/labels", query = ["label"])
    )]
    async fn labels(&self, label: Vec<String>) -> Result<Vec<String>, RpcError>;
}

struct FailingWireContract;

impl WireContract for FailingWireContract {
    async fn echo(&self, name: String) -> Result<String, RpcError> {
        Ok(name)
    }

    async fn create(
        &self,
        _id: String,
        _expand: Option<bool>,
        _request: CreateUser,
    ) -> Result<String, RpcError> {
        Err(RpcError::application(
            StatusCode::CONFLICT,
            "user_conflict",
            "the user already exists",
        )
        .expect("the fixture error is valid"))
    }

    async fn health(&self) -> Result<(), RpcError> {
        Ok(())
    }

    async fn unhealthy(&self) -> Result<(), RpcError> {
        Err(
            RpcError::application(StatusCode::CONFLICT, "unhealthy", "health check failed")
                .unwrap(),
        )
    }

    async fn filter(&self, enabled: Option<bool>) -> Result<Option<bool>, RpcError> {
        Ok(enabled)
    }

    async fn labels(&self, label: Vec<String>) -> Result<Vec<String>, RpcError> {
        Ok(label)
    }
}

#[derive(Debug)]
struct CapturedRequest {
    method: Method,
    version: Version,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fusen_v1_request_headers_and_envelopes_match_the_contract() {
    let (addr, mut captured, fixture) =
        spawn_h2_fixture(FUSEN_CONTENT_TYPE, br#"{"result":"fusen-response"}"#).await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .protocol(WireProtocol::FusenV1)
        .connect()
        .await
        .unwrap();

    assert_eq!(
        client.echo("Ada Lovelace".into()).await.unwrap(),
        "fusen-response"
    );
    let request = captured.recv().await.expect("fixture captured one request");

    assert_eq!(request.method, Method::POST);
    assert_eq!(request.version, Version::HTTP_2);
    assert_eq!(
        request.uri.path_and_query().unwrap().as_str(),
        "/_fusen/v1/wire-contract/echo"
    );
    assert_eq!(request.uri.scheme_str(), Some("http"));
    assert_eq!(
        request.headers.get(CONTENT_TYPE).unwrap(),
        FUSEN_CONTENT_TYPE
    );
    assert_eq!(
        request.headers.get("x-fusen-service-group").unwrap(),
        "prod"
    );
    assert_eq!(request.headers.get("x-fusen-service-version").unwrap(), "1");
    assert_eq!(request.headers.get("x-fusen-attempt").unwrap(), "1");
    assert_valid_request_id(&request.headers);
    assert_valid_timeout(&request.headers);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&request.body).unwrap(),
        json!({"arguments": {"name": "Ada Lovelace"}})
    );

    fixture.abort();
    let _ = fixture.await;
    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spring_cloud_v1_path_query_body_and_raw_success_match_the_contract() {
    let (addr, mut captured, fixture) =
        spawn_h1_fixture(JSON_CONTENT_TYPE, br#""spring-response""#).await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .protocol(WireProtocol::SpringCloudV1)
        .connect()
        .await
        .unwrap();

    let response = client
        .create(
            "Ada Lovelace/analytical engine".into(),
            Some(true),
            CreateUser {
                name: "Charles Babbage".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(response, "spring-response");
    let request = captured.recv().await.expect("fixture captured one request");

    assert_eq!(request.method, Method::POST);
    assert_eq!(request.version, Version::HTTP_11);
    assert_eq!(
        request.uri.path(),
        "/users/Ada%20Lovelace%2Fanalytical%20engine"
    );
    assert_eq!(request.uri.query(), Some("expand=true"));
    assert_eq!(
        request.headers.get(CONTENT_TYPE).unwrap(),
        JSON_CONTENT_TYPE
    );
    assert_eq!(request.headers.get("x-fusen-attempt").unwrap(), "1");
    assert!(request.headers.get("x-fusen-service-group").is_none());
    assert!(request.headers.get("x-fusen-service-version").is_none());
    assert_valid_request_id(&request.headers);
    assert_valid_timeout(&request.headers);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&request.body).unwrap(),
        json!({"name": "Charles Babbage"})
    );

    fixture.await.unwrap();
    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spring_cloud_v1_repeated_query_uses_one_key_per_value() {
    let (addr, mut captured, fixture) =
        spawn_h1_fixture(JSON_CONTENT_TYPE, br#"["one","two words"]"#).await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .protocol(WireProtocol::SpringCloudV1)
        .connect()
        .await
        .unwrap();

    let labels = vec!["one".to_owned(), "two words".to_owned()];
    assert_eq!(client.labels(labels.clone()).await.unwrap(), labels);
    let request = captured.recv().await.expect("fixture captured one request");

    assert_eq!(request.method, Method::GET);
    assert_eq!(request.version, Version::HTTP_11);
    assert_eq!(request.uri.path(), "/labels");
    assert_eq!(request.uri.query(), Some("label=one&label=two%20words"));
    assert!(request.body.is_empty());

    fixture.await.unwrap();
    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spring_cloud_v1_empty_repeated_query_omits_the_key() {
    let (addr, mut captured, fixture) = spawn_h1_fixture(JSON_CONTENT_TYPE, br#"[]"#).await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .protocol(WireProtocol::SpringCloudV1)
        .connect()
        .await
        .unwrap();

    assert!(client.labels(Vec::new()).await.unwrap().is_empty());
    let request = captured.recv().await.expect("fixture captured one request");
    assert_eq!(request.uri.path(), "/labels");
    assert_eq!(request.uri.query(), None);

    fixture.await.unwrap();
    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_rejects_duplicate_response_content_type() {
    let (addr, fixture) = spawn_h1_duplicate_content_type_fixture().await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .protocol(WireProtocol::SpringCloudV1)
        .connect()
        .await
        .unwrap();

    let error = client
        .echo("duplicate-content-type".into())
        .await
        .unwrap_err();
    assert_eq!(error.category(), RpcCategory::DataLoss);
    assert_eq!(error.code().as_str(), "invalid_content_type");

    fixture.await.unwrap();
    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spring_cloud_v1_head_uses_a_unit_success_contract() {
    let config = ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .service(WireContractServer::new(FailingWireContract))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .protocol(WireProtocol::SpringCloudV1)
        .connect()
        .await
        .unwrap();

    client.health().await.unwrap();
    let error = client.unhealthy().await.unwrap_err();
    assert_eq!(error.code().as_str(), "remote_head_error");
    assert_eq!(error.status(), StatusCode::CONFLICT);

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spring_cloud_v1_rejects_duplicate_scalar_query_parameters() {
    let config = ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .service(WireContractServer::new(FailingWireContract))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();

    let request = Request::builder()
        .method(Method::GET)
        .uri("/filters?enabled=true&enabled=false")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = send_h1(server.local_addr(), request).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers().get(CONTENT_TYPE).unwrap(),
        PROBLEM_CONTENT_TYPE
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let problem: ProblemDetails = serde_json::from_slice(&body).unwrap();
    assert_eq!(problem.code.as_str(), "duplicate_query_parameter");
    assert_eq!(problem.status, StatusCode::BAD_REQUEST.as_u16());

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn problem_details_preserve_category_code_request_id_and_retryability() {
    let config = ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .build()
        .unwrap();
    let server = Server::builder("127.0.0.1:0")
        .config(config)
        .service(WireContractServer::new(FailingWireContract))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();

    let request = Request::builder()
        .method(Method::POST)
        .uri("/users/conflict?expand=false")
        .header(CONTENT_TYPE, JSON_CONTENT_TYPE)
        .header("x-request-id", "problem-request-42")
        .header("x-fusen-timeout-ms", "5000")
        .header("x-fusen-attempt", "1")
        .body(Full::new(Bytes::from_static(br#"{"name":"Ada"}"#)))
        .unwrap();
    let response = send_h1(server.local_addr(), request).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response.headers().get(CONTENT_TYPE).unwrap(),
        PROBLEM_CONTENT_TYPE
    );
    assert_eq!(
        response.headers().get("x-request-id").unwrap(),
        "problem-request-42"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let problem: ProblemDetails = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        problem.type_uri,
        "urn:fusen:error:application:user_conflict"
    );
    assert_eq!(problem.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(problem.code.as_str(), "user_conflict");
    assert_eq!(problem.request_id, "problem-request-42");
    assert!(!problem.retryable);
    assert_eq!(problem.detail.as_deref(), Some("the user already exists"));
    assert_eq!(problem.instance.as_deref(), Some("/users/conflict"));

    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .protocol(WireProtocol::SpringCloudV1)
        .connect()
        .await
        .unwrap();
    let error = client
        .create(
            "conflict".into(),
            Some(false),
            CreateUser { name: "Ada".into() },
        )
        .await
        .expect_err("the application fixture always rejects create");
    assert_eq!(error.category(), RpcCategory::Application);
    assert_eq!(error.code().as_str(), "user_conflict");
    assert!(!error.retryable());

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

async fn spawn_h2_fixture(
    response_content_type: &'static str,
    response_body: &'static [u8],
) -> (
    SocketAddr,
    mpsc::UnboundedReceiver<CapturedRequest>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (captured_tx, captured_rx) = mpsc::unbounded_channel();
    let fixture = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let service = service_fn(move |request| {
            capture_and_respond(
                request,
                captured_tx.clone(),
                response_content_type,
                response_body,
            )
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });
    (addr, captured_rx, fixture)
}

async fn spawn_h1_fixture(
    response_content_type: &'static str,
    response_body: &'static [u8],
) -> (
    SocketAddr,
    mpsc::UnboundedReceiver<CapturedRequest>,
    JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (captured_tx, captured_rx) = mpsc::unbounded_channel();
    let fixture = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let service = service_fn(move |request| {
            capture_and_respond(
                request,
                captured_tx.clone(),
                response_content_type,
                response_body,
            )
        });
        let mut builder = hyper::server::conn::http1::Builder::new();
        builder.keep_alive(false);
        builder
            .serve_connection(TokioIo::new(stream), service)
            .await
            .unwrap();
    });
    (addr, captured_rx, fixture)
}

async fn spawn_h1_duplicate_content_type_fixture() -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let fixture = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let service = service_fn(|request: Request<Incoming>| async move {
            let _ = request.into_body().collect().await.unwrap();
            let mut response = Response::new(Full::new(Bytes::from_static(b"\"response\"")));
            response.headers_mut().append(
                CONTENT_TYPE,
                http::HeaderValue::from_static(JSON_CONTENT_TYPE),
            );
            response
                .headers_mut()
                .append(CONTENT_TYPE, http::HeaderValue::from_static("text/plain"));
            Ok::<_, Infallible>(response)
        });
        let mut builder = hyper::server::conn::http1::Builder::new();
        builder.keep_alive(false);
        builder
            .serve_connection(TokioIo::new(stream), service)
            .await
            .unwrap();
    });
    (addr, fixture)
}

async fn capture_and_respond(
    request: Request<Incoming>,
    captured: mpsc::UnboundedSender<CapturedRequest>,
    response_content_type: &'static str,
    response_body: &'static [u8],
) -> Result<Response<Full<Bytes>>, Infallible> {
    let (parts, body) = request.into_parts();
    let body = body.collect().await.unwrap().to_bytes();
    captured
        .send(CapturedRequest {
            method: parts.method,
            version: parts.version,
            uri: parts.uri,
            headers: parts.headers,
            body,
        })
        .unwrap();
    Ok(Response::builder()
        .header(CONTENT_TYPE, response_content_type)
        .body(Full::new(Bytes::from_static(response_body)))
        .unwrap())
}

async fn send_h1(addr: SocketAddr, request: Request<Full<Bytes>>) -> Response<Incoming> {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .unwrap();
    let connection = tokio::spawn(async move { connection.await.unwrap() });
    let response = sender.send_request(request).await.unwrap();
    drop(sender);
    connection.await.unwrap();
    response
}

fn assert_valid_request_id(headers: &HeaderMap) {
    let value = headers
        .get("x-request-id")
        .expect("client emits a request ID")
        .to_str()
        .unwrap();
    assert!((1..=64).contains(&value.len()));
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    );
}

fn assert_valid_timeout(headers: &HeaderMap) {
    let timeout = headers
        .get("x-fusen-timeout-ms")
        .expect("client emits its remaining deadline")
        .to_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(timeout <= 10_000);
}
