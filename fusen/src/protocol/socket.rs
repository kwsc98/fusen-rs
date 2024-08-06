use std::convert::Infallible;

use fusen_common::error::FusenError;
use http::{Request, Response, Uri, Version};
use http_body_util::combinators::BoxBody;
use hyper::body::Incoming;
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::{connect::HttpConnector, Client};
use tracing::error;
pub type HttpSocket = Client<HttpsConnector<HttpConnector>, BoxBody<bytes::Bytes, Infallible>>;
use crate::register::Resource;

#[derive(Debug)]
pub struct InvokerAssets {
    pub resource: Resource,
    pub socket: Socket,
}

#[derive(Debug)]
pub enum Socket {
    HTTP1(HttpSocket),
    HTTP2(HttpSocket),
}

impl Socket {
    pub fn new(protocol: Option<&str>) -> Self {
        if protocol.is_some_and(|e| e.to_lowercase().contains("http2")) {
            Socket::HTTP2(
                Client::builder(hyper_util::rt::TokioExecutor::new())
                    .http2_adaptive_window(true)
                    .http2_only(true)
                    .build(HttpsConnector::new()),
            )
        } else {
            Socket::HTTP1(
                Client::builder(hyper_util::rt::TokioExecutor::new()).build(HttpsConnector::new()),
            )
        }
    }
}

impl InvokerAssets {
    pub async fn send_request(
        &self,
        mut request: Request<BoxBody<bytes::Bytes, Infallible>>,
    ) -> Result<Response<Incoming>, FusenError> {
        match &self.socket {
            Socket::HTTP1(client) => {
                *request.version_mut() = Version::HTTP_11;
                send_http_request(client, &self.resource, request).await
            }
            Socket::HTTP2(client) => {
                *request.version_mut() = Version::HTTP_2;
                send_http_request(client, &self.resource, request).await
            }
        }
    }
}

async fn send_http_request(
    client: &Client<HttpsConnector<HttpConnector>, BoxBody<bytes::Bytes, Infallible>>,
    resource: &Resource,
    mut request: Request<BoxBody<bytes::Bytes, Infallible>>,
) -> Result<Response<Incoming>, FusenError> {
    let org: &Uri = request.uri();
    let temp_url: Uri = resource.get_addr().parse().unwrap();
    let mut host = temp_url.host().unwrap().to_string();
    if let Some(port) = temp_url.port_u16() {
        host.push_str(&format!(":{}", port));
    }
    let new_uri = Uri::builder()
        .scheme(temp_url.scheme_str().unwrap_or("http"))
        .authority(host)
        .path_and_query(org.path_and_query().map_or("", |e| e.as_str()))
        .build()?;
    *request.uri_mut() = new_uri;
    let response = client.request(request).await.map_err(|e| {
        error!("error : {:?}", e);
        FusenError::from(e.to_string())
    })?;
    Ok(response)
}
