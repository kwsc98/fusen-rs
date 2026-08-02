//! Binding-specific media type preflight contracts.

use bytes::Bytes;
use fusen_rs::{
    Body, BufferedResponse, ClientErrorKind, ClientRuntime, EncodedRequest, Error, ErrorCategory,
    ErrorDecoder, HttpBindingId, RequestEncoder, RequestEncoding, Response, ResponseDecoder,
    Server, ServerErrorKind, interface,
};
use http::{HeaderMap, Method};

#[interface(name = "text-contract")]
trait TextApi {
    #[fusen_rs::method(
        method = "POST",
        path = "/text",
        consumes = "text/plain",
        produces = "text/plain"
    )]
    async fn text(&self, #[param(body)] value: String) -> Result<Response<String>, Error>;
}

#[interface(name = "text-response-contract")]
trait TextResponseApi {
    #[fusen_rs::method(
        method = "GET",
        path = "/text-response",
        consumes = "application/json",
        produces = "text/plain"
    )]
    async fn text_response(&self) -> Result<Response<String>, Error>;
}

struct TextResponseHandler;

impl TextResponseApi for TextResponseHandler {
    async fn text_response(&self) -> Result<Response<String>, Error> {
        Ok(Response::new("text".to_owned()))
    }
}

struct TextHandler;

impl TextApi for TextHandler {
    async fn text(&self, value: String) -> Result<Response<String>, Error> {
        Ok(Response::new(value))
    }
}

#[interface(name = "vendor-json-contract")]
trait VendorJsonApi {
    #[fusen_rs::method(
        method = "POST",
        path = "/vendor-json",
        consumes = "application/vnd.example.request+json; charset=utf-8",
        produces = "application/problem+json"
    )]
    async fn invoke(&self, #[param(body)] value: String) -> Result<Response<String>, Error>;
}

struct VendorJsonHandler;

impl VendorJsonApi for VendorJsonHandler {
    async fn invoke(&self, value: String) -> Result<Response<String>, Error> {
        Ok(Response::new(value))
    }
}

struct TextCodec;

impl RequestEncoder for TextCodec {
    fn encode(&self, _request: RequestEncoding<'_>) -> Result<EncodedRequest, Error> {
        Ok(EncodedRequest::new(
            Method::POST,
            "/text",
            HeaderMap::new(),
            Bytes::new(),
        ))
    }
}

impl ResponseDecoder for TextCodec {
    fn decode(
        &self,
        _method: &'static fusen_rs::contract::MethodDescriptor,
        response: BufferedResponse,
    ) -> Result<Response<Body>, Error> {
        Ok(Response::new(Body::from_bytes(response.body().clone())))
    }
}

impl ErrorDecoder for TextCodec {
    fn decode(
        &self,
        _method: &'static fusen_rs::contract::MethodDescriptor,
        _response: BufferedResponse,
    ) -> Error {
        Error::local(
            ErrorCategory::Internal,
            "text_binding_error",
            "text binding rejected the response",
        )
        .unwrap()
    }
}

#[tokio::test]
async fn built_in_json_client_rejects_non_json_contract_before_io() {
    let runtime = ClientRuntime::builder().build().unwrap();
    let error = match TextApiClient::builder(&runtime)
        .direct("http://127.0.0.1:1")
        .connect()
        .await
    {
        Ok(_) => panic!("http-json-v1 must reject text/plain during connect"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ClientErrorKind::Connect);
    assert!(error.message().contains("http-json-v1"));
    assert!(error.message().contains("text/plain"));
    runtime.shutdown().await.unwrap();
}

#[test]
fn built_in_json_server_rejects_non_json_contract_before_bind() {
    let error = match Server::builder("127.0.0.1:0")
        .interface(TextApiServer::new(TextHandler))
        .build()
    {
        Ok(_) => panic!("the built-in server must reject text/plain during build"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ServerErrorKind::Validation);
    assert!(error.message().contains("http-json-v1"));
    assert!(error.message().contains("text/plain"));
}

#[tokio::test]
async fn built_in_json_client_rejects_non_json_produces_before_io() {
    let runtime = ClientRuntime::builder().build().unwrap();
    let error = match TextResponseApiClient::builder(&runtime)
        .direct("http://127.0.0.1:1")
        .connect()
        .await
    {
        Ok(_) => panic!("http-json-v1 must reject a text/plain response contract"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ClientErrorKind::Connect);
    assert!(error.message().contains("produces"));
    assert!(error.message().contains("text/plain"));
    runtime.shutdown().await.unwrap();
}

#[test]
fn built_in_json_server_rejects_non_json_produces_before_bind() {
    let error = match Server::builder("127.0.0.1:0")
        .interface(TextResponseApiServer::new(TextResponseHandler))
        .build()
    {
        Ok(_) => panic!("the built-in server must reject a text/plain response contract"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ServerErrorKind::Validation);
    assert!(error.message().contains("produces"));
    assert!(error.message().contains("text/plain"));
}

#[tokio::test]
async fn custom_client_binding_accepts_a_non_json_contract() {
    let binding = HttpBindingId::new("http-text-v1").unwrap();
    let runtime = ClientRuntime::builder()
        .http_binding(binding.clone(), TextCodec, TextCodec, TextCodec)
        .build()
        .unwrap();
    let client = TextApiClient::builder(&runtime)
        .binding(binding)
        .direct("http://127.0.0.1:1")
        .connect()
        .await
        .expect("custom bindings own their media type contract");

    drop(client);
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn built_in_json_binding_accepts_json_family_media_types() {
    let runtime = ClientRuntime::builder().build().unwrap();
    let client = VendorJsonApiClient::builder(&runtime)
        .direct("http://127.0.0.1:1")
        .connect()
        .await
        .expect("vendor and problem JSON media types are JSON-compatible");
    let server = Server::builder("127.0.0.1:0")
        .interface(VendorJsonApiServer::new(VendorJsonHandler))
        .build()
        .expect("the built-in server accepts JSON-family media types");

    drop(server);
    drop(client);
    runtime.shutdown().await.unwrap();
}
