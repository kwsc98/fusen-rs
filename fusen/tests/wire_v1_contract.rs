//! Golden contract coverage for the built-in HTTP JSON binding.

use bytes::Bytes;
use fusen_rs::{
    ClientRuntime, Error, ErrorCategory, ErrorDetails, ErrorKind, ErrorOrigin, Response,
    SensitiveFields, Server,
    contract::{EndpointCapabilities, HttpBindingId, HttpVersionPolicy, HttpVersionSet},
    interface,
};
use http::{
    HeaderMap, Method, Request, Response as HttpResponse, StatusCode, Uri, Version,
    header::CONTENT_TYPE,
};
use http_body_util::{BodyExt, Full};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{convert::Infallible, net::SocketAddr, time::Duration};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};

const JSON_CONTENT_TYPE: &str = "application/json";
const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";

#[derive(Deserialize)]
struct WireProblemDetails {
    #[serde(rename = "type")]
    type_uri: String,
    status: u16,
    detail: Option<String>,
    instance: Option<String>,
    code: String,
    request_id: Option<String>,
    retryable: bool,
    details: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, SensitiveFields)]
struct CreateUser {
    name: String,
}

type EnabledAlias = bool;
type LabelsAlias = Vec<String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, SensitiveFields)]
#[serde(transparent)]
struct CountAlias(u64);

#[interface(name = "wire-contract", group = "prod", version = "1")]
trait WireContract {
    #[fusen_rs::method(method = "GET", path = "/echo/{name}")]
    async fn echo(&self, #[param(path)] name: String) -> Result<Response<String>, Error>;

    #[fusen_rs::method(method = "POST", path = "/users/{id}")]
    async fn create(
        &self,
        id: String,
        #[param(query)] expand: Option<bool>,
        #[param(body)] request: CreateUser,
    ) -> Result<Response<String>, Error>;

    #[fusen_rs::method(method = "HEAD", path = "/health")]
    async fn health(&self) -> Result<Response<()>, Error>;

    #[fusen_rs::method(method = "HEAD", path = "/unhealthy")]
    async fn unhealthy(&self) -> Result<Response<()>, Error>;

    #[fusen_rs::method(method = "GET", path = "/filters")]
    async fn filter(
        &self,
        #[param(query)] enabled: Option<bool>,
    ) -> Result<Response<Option<bool>>, Error>;

    #[fusen_rs::method(method = "GET", path = "/labels")]
    async fn labels(
        &self,
        #[param(query, repeated)] label: Vec<String>,
    ) -> Result<Response<Vec<String>>, Error>;

    #[fusen_rs::method(method = "GET", path = "/aliases/count/{count}")]
    async fn alias_count(
        &self,
        #[param(path)] count: CountAlias,
    ) -> Result<Response<CountAlias>, Error>;

    #[fusen_rs::method(method = "GET", path = "/aliases/filter")]
    async fn alias_filter(
        &self,
        #[param(query)] enabled: EnabledAlias,
    ) -> Result<Response<EnabledAlias>, Error>;

    #[fusen_rs::method(method = "GET", path = "/aliases/labels")]
    async fn alias_labels(
        &self,
        #[param(query, repeated)] label: LabelsAlias,
    ) -> Result<Response<LabelsAlias>, Error>;

    #[fusen_rs::method(method = "GET", path = "/aliases/scalar-labels")]
    async fn alias_labels_declared_scalar(
        &self,
        #[param(query)] label: LabelsAlias,
    ) -> Result<Response<LabelsAlias>, Error>;

    #[fusen_rs::method(method = "GET", path = "/aliases/repeated-filter")]
    async fn alias_filter_declared_repeated(
        &self,
        #[param(query, repeated)] enabled: EnabledAlias,
    ) -> Result<Response<EnabledAlias>, Error>;
}

struct FailingWireContract;

impl WireContract for FailingWireContract {
    async fn echo(&self, name: String) -> Result<Response<String>, Error> {
        Ok(Response::new(name))
    }

    async fn create(
        &self,
        _id: String,
        _expand: Option<bool>,
        _request: CreateUser,
    ) -> Result<Response<String>, Error> {
        let mut details = ErrorDetails::new();
        details.insert("field", json!("id"));
        details.insert("constraint", json!("unique"));
        let mut error = Error::application(
            ErrorCategory::Conflict,
            "user_conflict",
            "the user already exists",
        )
        .expect("the fixture error is valid")
        .with_details(details);
        error
            .headers_mut()
            .insert("x-error-scope", "user".parse().unwrap());
        Err(error)
    }

    async fn health(&self) -> Result<Response<()>, Error> {
        Ok(Response::new(()))
    }

    async fn unhealthy(&self) -> Result<Response<()>, Error> {
        Err(
            Error::application(ErrorCategory::Conflict, "unhealthy", "health check failed")
                .unwrap(),
        )
    }

    async fn filter(&self, enabled: Option<bool>) -> Result<Response<Option<bool>>, Error> {
        Ok(Response::new(enabled))
    }

    async fn labels(&self, label: Vec<String>) -> Result<Response<Vec<String>>, Error> {
        Ok(Response::new(label))
    }

    async fn alias_count(&self, count: CountAlias) -> Result<Response<CountAlias>, Error> {
        Ok(Response::new(count))
    }

    async fn alias_filter(&self, enabled: EnabledAlias) -> Result<Response<EnabledAlias>, Error> {
        Ok(Response::new(enabled))
    }

    async fn alias_labels(&self, label: LabelsAlias) -> Result<Response<LabelsAlias>, Error> {
        Ok(Response::new(label))
    }

    async fn alias_labels_declared_scalar(
        &self,
        label: LabelsAlias,
    ) -> Result<Response<LabelsAlias>, Error> {
        Ok(Response::new(label))
    }

    async fn alias_filter_declared_repeated(
        &self,
        enabled: EnabledAlias,
    ) -> Result<Response<EnabledAlias>, Error> {
        Ok(Response::new(enabled))
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
async fn h2c_path_query_body_and_invocation_controls_match_the_contract() {
    let (addr, mut captured, fixture) =
        spawn_h2_fixture(JSON_CONTENT_TYPE, br#""h2c-response""#).await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .binding(HttpBindingId::default())
        .http_version_policy(HttpVersionPolicy::H2c)
        .direct_capabilities(endpoint_capabilities(HttpVersionSet::HTTP_2))
        .connect()
        .await
        .unwrap();

    assert_eq!(
        client
            .create(
                "user-7".into(),
                Some(true),
                CreateUser {
                    name: "Ada Lovelace".into(),
                },
            )
            .await
            .unwrap()
            .into_body(),
        "h2c-response"
    );
    let request = captured.recv().await.expect("fixture captured one request");

    assert_eq!(request.method, Method::POST);
    assert_eq!(request.version, Version::HTTP_2);
    assert_eq!(
        request.uri.path_and_query().unwrap().as_str(),
        "/users/user-7?expand=true"
    );
    assert_eq!(request.uri.scheme_str(), Some("http"));
    assert_eq!(
        request.headers.get(CONTENT_TYPE).unwrap(),
        JSON_CONTENT_TYPE
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
        json!({"name": "Ada Lovelace"})
    );

    fixture.abort();
    let _ = fixture.await;
    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http1_path_query_body_and_raw_success_match_the_contract() {
    let (addr, mut captured, fixture) =
        spawn_h1_fixture(JSON_CONTENT_TYPE, br#""http1-response""#).await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .http_version_policy(HttpVersionPolicy::Http1)
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
    assert_eq!(response.into_body(), "http1-response");
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
    assert!(request.headers.get("x-fusen-attempt").is_none());
    assert!(request.headers.get("x-fusen-service-group").is_none());
    assert!(request.headers.get("x-fusen-service-version").is_none());
    assert!(request.headers.get("x-request-id").is_none());
    assert!(request.headers.get("x-fusen-timeout-ms").is_none());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&request.body).unwrap(),
        json!({"name": "Charles Babbage"})
    );

    fixture.await.unwrap();
    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_query_uses_one_key_per_value() {
    let (addr, mut captured, fixture) =
        spawn_h1_fixture(JSON_CONTENT_TYPE, br#"["one","two words"]"#).await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .connect()
        .await
        .unwrap();

    let labels = vec!["one".to_owned(), "two words".to_owned()];
    assert_eq!(
        client.labels(labels.clone()).await.unwrap().into_body(),
        labels
    );
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
async fn single_repeated_query_keeps_the_array_contract() {
    let (addr, mut captured, fixture) = spawn_h1_fixture(JSON_CONTENT_TYPE, br#"["one"]"#).await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .connect()
        .await
        .unwrap();

    assert_eq!(
        client
            .labels(vec!["one".to_owned()])
            .await
            .unwrap()
            .into_body(),
        ["one"]
    );
    let request = captured.recv().await.expect("fixture captured one request");
    assert_eq!(request.uri.path(), "/labels");
    assert_eq!(request.uri.query(), Some("label=one"));

    fixture.await.unwrap();
    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_repeated_query_omits_the_key() {
    let (addr, mut captured, fixture) = spawn_h1_fixture(JSON_CONTENT_TYPE, br#"[]"#).await;
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{addr}"))
        .connect()
        .await
        .unwrap();

    assert!(
        client
            .labels(Vec::new())
            .await
            .unwrap()
            .into_body()
            .is_empty()
    );
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
        .connect()
        .await
        .unwrap();

    let error = client
        .echo("duplicate-content-type".into())
        .await
        .unwrap_err();
    assert_eq!(error.category(), ErrorCategory::DataLoss);
    assert_eq!(error.code().as_str(), "invalid_content_type");

    fixture.await.unwrap();
    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_uses_a_unit_success_contract() {
    let server = Server::builder("127.0.0.1:0")
        .interface(WireContractServer::new(FailingWireContract))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
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
async fn duplicate_scalar_query_parameters_are_rejected() {
    let server = Server::builder("127.0.0.1:0")
        .interface(WireContractServer::new(FailingWireContract))
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
    let problem: WireProblemDetails = serde_json::from_slice(&body).unwrap();
    assert_eq!(problem.code.as_str(), "duplicate_query_parameter");
    assert_eq!(problem.status, StatusCode::BAD_REQUEST.as_u16());

    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scalars_are_decoded_using_the_declared_dto_type() {
    let server = Server::builder("127.0.0.1:0")
        .interface(WireContractServer::new(FailingWireContract))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();

    let request = Request::builder()
        .method(Method::GET)
        .uri("/echo/0")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = send_h1(server.local_addr(), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        json!("0")
    );

    let request = Request::builder()
        .method(Method::GET)
        .uri("/filters?enabled=true")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let response = send_h1(server.local_addr(), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        json!(true)
    );

    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .connect()
        .await
        .unwrap();
    assert_eq!(
        client
            .alias_count(CountAlias(42))
            .await
            .unwrap()
            .into_body(),
        CountAlias(42)
    );
    assert!(client.alias_filter(true).await.unwrap().into_body());
    assert_eq!(
        client
            .alias_labels(vec!["one".to_owned(), "two".to_owned()])
            .await
            .unwrap()
            .into_body(),
        ["one", "two"]
    );

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn descriptor_shape_mismatches_are_rejected_before_network_io() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{}", listener.local_addr().unwrap()))
        .connect()
        .await
        .unwrap();

    let scalar_array = client
        .alias_labels_declared_scalar(vec!["one".to_owned()])
        .await
        .unwrap_err();
    assert_eq!(scalar_array.category(), ErrorCategory::InvalidArgument);
    assert_eq!(scalar_array.code().as_str(), "invalid_http_parameter");
    assert!(scalar_array.message().contains("label"));
    assert!(scalar_array.message().contains("#[param(query, repeated)]"));

    let repeated_scalar = client
        .alias_filter_declared_repeated(true)
        .await
        .unwrap_err();
    assert_eq!(repeated_scalar.category(), ErrorCategory::InvalidArgument);
    assert_eq!(repeated_scalar.code().as_str(), "invalid_http_parameter");
    assert!(repeated_scalar.message().contains("enabled"));
    assert!(repeated_scalar.message().contains("remove `repeated`"));

    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err(),
        "invalid HTTP parameter shapes must fail before opening a connection"
    );

    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn problem_details_preserve_category_code_request_id_and_retryability() {
    let server = Server::builder("127.0.0.1:0")
        .interface(WireContractServer::new(FailingWireContract))
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
        .header("x-fusen-service-group", "prod")
        .header("x-fusen-service-version", "1")
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
    assert_eq!(response.headers().get("x-error-scope").unwrap(), "user");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let problem: WireProblemDetails = serde_json::from_slice(&body).unwrap();

    assert_eq!(
        problem.type_uri,
        "urn:fusen:error:application:user_conflict"
    );
    assert_eq!(problem.status, StatusCode::CONFLICT.as_u16());
    assert_eq!(problem.code.as_str(), "user_conflict");
    assert_eq!(problem.request_id.as_deref(), Some("problem-request-42"));
    assert!(!problem.retryable);
    assert_eq!(problem.detail.as_deref(), Some("the user already exists"));
    assert_eq!(problem.instance.as_deref(), Some("/users/conflict"));
    assert_eq!(problem.details.as_ref().unwrap()["field"], json!("id"));
    assert_eq!(
        problem.details.as_ref().unwrap()["constraint"],
        json!("unique")
    );
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = WireContractClient::builder(&runtime)
        .direct(format!("http://{}", server.local_addr()))
        .direct_capabilities(endpoint_capabilities(HttpVersionSet::ALL))
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
    assert_eq!(error.kind(), ErrorKind::Application);
    assert_eq!(error.category(), ErrorCategory::Conflict);
    assert_eq!(error.origin(), ErrorOrigin::Remote);
    assert!(error.request_id().is_some());
    assert_eq!(error.code().as_str(), "user_conflict");
    assert!(!error.retry_hint().is_retryable());
    assert_eq!(error.headers().get("x-error-scope").unwrap(), "user");
    assert_eq!(error.details().unwrap().get("field"), Some(&json!("id")));

    drop(client);
    runtime.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invocation_controls_must_match_the_routed_service_selector() {
    let server = Server::builder("127.0.0.1:0")
        .interface(WireContractServer::new(FailingWireContract))
        .build()
        .unwrap()
        .start()
        .await
        .unwrap();
    let addr = server.local_addr();

    let response = send_h1(
        addr,
        echo_request(&[
            ("x-request-id", "selector-match"),
            ("x-fusen-timeout-ms", "5000"),
            ("x-fusen-attempt", "1"),
            ("x-fusen-service-group", "prod"),
            ("x-fusen-service-version", "1"),
        ]),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let response = send_h1(addr, echo_request(&[])).await;
    assert_eq!(response.status(), StatusCode::OK);

    for (headers, expected_code) in [
        (
            vec![
                ("x-request-id", "wrong-group"),
                ("x-fusen-service-group", "staging"),
                ("x-fusen-service-version", "1"),
            ],
            "service_group_mismatch",
        ),
        (
            vec![
                ("x-request-id", "missing-group"),
                ("x-fusen-service-version", "1"),
            ],
            "service_group_mismatch",
        ),
        (
            vec![
                ("x-request-id", "wrong-version"),
                ("x-fusen-service-group", "prod"),
                ("x-fusen-service-version", "2"),
            ],
            "service_version_mismatch",
        ),
        (
            vec![
                ("x-request-id", "missing-version"),
                ("x-fusen-service-group", "prod"),
            ],
            "service_version_mismatch",
        ),
        (
            vec![
                ("x-request-id", "duplicate-version"),
                ("x-fusen-service-group", "prod"),
                ("x-fusen-service-version", "1"),
                ("x-fusen-service-version", "1"),
            ],
            "service_version_mismatch",
        ),
    ] {
        let response = send_h1(addr, echo_request(&headers)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let problem: WireProblemDetails = serde_json::from_slice(&body).unwrap();
        assert_eq!(problem.code, expected_code);
    }

    server.shutdown().await.unwrap();
}

fn endpoint_capabilities(http_versions: HttpVersionSet) -> EndpointCapabilities {
    EndpointCapabilities::new(http_versions, [HttpBindingId::default()], true).unwrap()
}

fn echo_request(headers: &[(&str, &str)]) -> Request<Full<Bytes>> {
    let mut request = Request::builder().method(Method::GET).uri("/echo/Ada");
    for &(name, value) in headers {
        request = request.header(name, value);
    }
    request.body(Full::new(Bytes::new())).unwrap()
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
            let mut response = HttpResponse::new(Full::new(Bytes::from_static(b"\"response\"")));
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
) -> Result<HttpResponse<Full<Bytes>>, Infallible> {
    let (parts, body) = request.into_parts();
    let request_id = parts.headers.get("x-request-id").cloned();
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
    let mut response = HttpResponse::builder()
        .header(CONTENT_TYPE, response_content_type)
        .body(Full::new(Bytes::from_static(response_body)))
        .unwrap();
    if let Some(request_id) = request_id {
        response.headers_mut().insert("x-request-id", request_id);
    }
    Ok(response)
}

async fn send_h1(addr: SocketAddr, request: Request<Full<Bytes>>) -> HttpResponse<Incoming> {
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
