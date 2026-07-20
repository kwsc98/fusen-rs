use crate::error::FusenError;
use http::{Request, Response, Version};
use http_body_util::combinators::BoxBody;
use hyper::body::Incoming;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use std::{convert::Infallible, time::Duration};

pub type HttpSocket = Client<HttpsConnector<HttpConnector>, BoxBody<bytes::Bytes, Infallible>>;

#[derive(Debug)]
pub struct HttpClient {
    http1_client: HttpSocket,
    http2_client: HttpSocket,
}

impl HttpClient {
    pub fn new(connect_timeout: Duration) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(connect_timeout));
        connector.set_keepalive(Some(Duration::from_secs(90)));
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
        let client = match request.version() {
            Version::HTTP_2 => &self.http2_client,
            _ => &self.http1_client,
        };
        client
            .request(request)
            .await
            .map_err(|error| FusenError::internal("HTTP request failed", error))
    }
}
