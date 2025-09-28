use http::{Request, Response, Version};
use http_body_util::combinators::BoxBody;
use hyper::body::Incoming;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use std::{convert::Infallible, time::Duration};

use crate::error::FusenError;
pub type HttpSocket = Client<HttpsConnector<HttpConnector>, BoxBody<bytes::Bytes, Infallible>>;

pub enum HttpVersion {
    H1,
    H2,
}

#[derive(Debug)]
pub struct HttpClient {
    pub http1_client: HttpSocket,
    pub http2_client: HttpSocket,
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    pub fn new() -> Self {
        let mut connector = HttpConnector::new();
        connector.set_keepalive(Some(Duration::from_secs(1800)));
        connector.enforce_http(false);
        Self {
            http1_client: Client::builder(hyper_util::rt::TokioExecutor::new())
                .build(HttpsConnector::new_with_connector(connector.clone())),
            http2_client: Client::builder(hyper_util::rt::TokioExecutor::new())
                .http2_only(true)
                .build(HttpsConnector::new_with_connector(connector)),
        }
    }
    pub async fn send_http_request(
        &self,
        request: Request<BoxBody<bytes::Bytes, Infallible>>,
    ) -> Result<Response<Incoming>, FusenError> {
        let http_client = match request.version() {
            Version::HTTP_2 => &self.http2_client,
            _ => &self.http1_client,
        };
        send_http_request(http_client, request).await
    }
}

async fn send_http_request(
    client: &Client<HttpsConnector<HttpConnector>, BoxBody<bytes::Bytes, Infallible>>,
    request: Request<BoxBody<bytes::Bytes, Infallible>>,
) -> Result<Response<Incoming>, FusenError> {
    let response = client
        .request(request)
        .await
        .map_err(|error| FusenError::Error(Box::new(error)))?;
    Ok(response)
}
