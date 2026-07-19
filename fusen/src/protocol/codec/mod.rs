use crate::{
    error::FusenError,
    protocol::{
        codec::body::{RequestBodyCodec, ResponseBodyCodec, json::JsonCodec},
        fusen::{
            request::{FusenRequest, Path},
            response::{FusenResponse, HttpStatus},
        },
    },
};
use bytes::{Bytes, BytesMut};
use fusen_internal_common::protocol::WireProtocol;
use http::{
    Request, Response, Version,
    header::{CONNECTION, CONTENT_TYPE},
};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use std::{collections::HashMap, convert::Infallible};

pub mod body;

pub const DEFAULT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

pub struct FusenHttpCodec {
    json_codec: JsonCodec,
    max_body_bytes: usize,
}

impl Default for FusenHttpCodec {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_BODY_BYTES)
    }
}

impl FusenHttpCodec {
    pub fn new(max_body_bytes: usize) -> Self {
        Self {
            json_codec: JsonCodec,
            max_body_bytes,
        }
    }
}

impl RequestCodec<Bytes, hyper::Error> for FusenHttpCodec {
    fn encode(
        &self,
        fusen_request: &mut FusenRequest,
    ) -> Result<Request<BoxBody<Bytes, Infallible>>, FusenError> {
        let mut builder = Request::builder().header(CONNECTION, "keep-alive");
        for (key, value) in fusen_request.headers.drain() {
            builder = builder.header(key, value);
        }
        let addr = fusen_request
            .addr
            .as_deref()
            .ok_or_else(|| FusenError::ServiceUnavailable("request endpoint is missing".into()))?;
        let mut uri = format!("{}{}", addr.trim_end_matches('/'), fusen_request.path.path);
        if !fusen_request.querys.is_empty() {
            if fusen_request.path.path.contains('{') {
                let mut path = fusen_request.path.path.clone();
                for (key, value) in &fusen_request.querys {
                    path = path.replace(&format!("{{{key}}}"), &urlencoding::encode(value));
                }
                uri = format!("{}{path}", addr.trim_end_matches('/'));
            } else {
                let query = fusen_request
                    .querys
                    .iter()
                    .map(|(key, value)| {
                        format!(
                            "{}={}",
                            urlencoding::encode(key),
                            urlencoding::encode(value)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("&");
                uri.push('?');
                uri.push_str(&query);
            }
        }
        let mut body = Bytes::new();
        if let Some(bodies) = fusen_request.bodys.take() {
            builder = builder.header(CONTENT_TYPE, "application/json");
            body = RequestBodyCodec::encode(&self.json_codec, bodies)?;
        }
        let version = match fusen_request.protocol {
            WireProtocol::Fusen => Version::HTTP_2,
            WireProtocol::SpringCloud => Version::HTTP_11,
        };
        builder
            .version(version)
            .method(fusen_request.path.method.clone())
            .uri(uri)
            .body(Full::new(body).boxed())
            .map_err(|error| FusenError::internal("failed to build HTTP request", error))
    }

    async fn decode(
        &self,
        mut request: Request<BoxBody<Bytes, hyper::Error>>,
    ) -> Result<FusenRequest, FusenError> {
        let mut querys = HashMap::new();
        if let Some(request_querys) = request.uri().query() {
            for query in request_querys.split('&') {
                if let Some((key, value)) = query.split_once('=') {
                    let key = urlencoding::decode(key)
                        .map_err(|error| FusenError::InvalidRequest(error.to_string()))?
                        .into_owned();
                    let value = urlencoding::decode(value)
                        .map_err(|error| FusenError::InvalidRequest(error.to_string()))?
                        .into_owned();
                    querys.insert(key, value);
                }
            }
        }
        let headers = drain_headers(request.headers_mut());
        let content_type = headers.get(CONTENT_TYPE.as_str()).map(String::as_str);
        if content_type.is_some_and(|value| value.starts_with("application/grpc")) {
            return Err(FusenError::UnsupportedProtocol(
                "Dubbo Triple is disabled in 0.9".into(),
            ));
        }
        let bodys = if content_type.is_some_and(is_json_content_type) {
            let bytes = read_body(request.body_mut(), self.max_body_bytes).await?;
            Some(RequestBodyCodec::decode(&self.json_codec, bytes)?)
        } else {
            None
        };
        Ok(FusenRequest {
            path: Path {
                method: request.method().clone(),
                path: request.uri().path().to_owned(),
            },
            addr: None,
            querys,
            headers,
            extensions: None,
            bodys,
            protocol: if request.version() == Version::HTTP_2 {
                WireProtocol::Fusen
            } else {
                WireProtocol::SpringCloud
            },
        })
    }
}

impl ResponseCodec<Bytes, hyper::Error> for FusenHttpCodec {
    fn encode(
        &self,
        fusen_response: &mut FusenResponse,
    ) -> Result<Response<BoxBody<Bytes, Infallible>>, FusenError> {
        let mut builder = Response::builder();
        for (key, value) in fusen_response.headers.drain() {
            builder = builder.header(key, value);
        }
        let body = if let Some(value) = fusen_response.body.take() {
            builder = builder.header(CONTENT_TYPE, "application/json");
            ResponseBodyCodec::encode(&self.json_codec, value)?
        } else {
            Bytes::new()
        };
        builder
            .status(fusen_response.http_status.status)
            .body(Full::new(body).boxed())
            .map_err(|error| FusenError::internal("failed to build HTTP response", error))
    }

    async fn decode(
        &self,
        mut response: Response<BoxBody<Bytes, hyper::Error>>,
    ) -> Result<FusenResponse, FusenError> {
        let headers = drain_headers(response.headers_mut());
        let body = if headers
            .get(CONTENT_TYPE.as_str())
            .is_some_and(|value| is_json_content_type(value))
        {
            let bytes = read_body(response.body_mut(), self.max_body_bytes).await?;
            if bytes.is_empty() {
                None
            } else {
                Some(ResponseBodyCodec::decode(&self.json_codec, bytes)?)
            }
        } else {
            None
        };
        Ok(FusenResponse {
            protocol: if response.version() == Version::HTTP_2 {
                WireProtocol::Fusen
            } else {
                WireProtocol::SpringCloud
            },
            http_status: HttpStatus {
                status: response.status().as_u16(),
                message: None,
            },
            headers,
            extensions: None,
            body,
        })
    }
}

fn drain_headers(headers: &mut http::HeaderMap) -> HashMap<String, String> {
    headers
        .drain()
        .filter_map(|(key, value)| {
            key.map(|key| {
                (
                    key.to_string().to_ascii_lowercase(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
        })
        .collect()
}

fn is_json_content_type(value: &str) -> bool {
    value.starts_with("application/json") || value.starts_with("application/problem+json")
}

#[allow(async_fn_in_trait)]
pub trait RequestCodec<T, E> {
    fn encode(
        &self,
        request: &mut FusenRequest,
    ) -> Result<Request<BoxBody<T, Infallible>>, FusenError>;
    async fn decode(&self, request: Request<BoxBody<T, E>>) -> Result<FusenRequest, FusenError>;
}

#[allow(async_fn_in_trait)]
pub trait ResponseCodec<T, E> {
    fn encode(
        &self,
        response: &mut FusenResponse,
    ) -> Result<Response<BoxBody<T, Infallible>>, FusenError>;
    async fn decode(&self, response: Response<BoxBody<T, E>>) -> Result<FusenResponse, FusenError>;
}

async fn read_body(
    body: &mut BoxBody<Bytes, hyper::Error>,
    limit: usize,
) -> Result<Bytes, FusenError> {
    let mut bytes = BytesMut::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| FusenError::InvalidRequest(error.to_string()))?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > limit {
                return Err(FusenError::PayloadTooLarge { limit });
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(bytes.freeze())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(value: &'static [u8]) -> BoxBody<Bytes, hyper::Error> {
        Full::new(Bytes::from_static(value))
            .map_err(|never: Infallible| -> hyper::Error { match never {} })
            .boxed()
    }

    #[tokio::test]
    async fn rejects_oversized_body() {
        let codec = FusenHttpCodec::new(3);
        let request = Request::builder()
            .method("POST")
            .uri("/demo")
            .header(CONTENT_TYPE, "application/json")
            .body(body(b"1234"))
            .unwrap();
        let error = RequestCodec::decode(&codec, request).await.unwrap_err();
        assert!(matches!(error, FusenError::PayloadTooLarge { limit: 3 }));
    }

    #[tokio::test]
    async fn rejects_grpc_content_type() {
        let codec = FusenHttpCodec::default();
        let request = Request::builder()
            .method("POST")
            .uri("/demo")
            .header(CONTENT_TYPE, "application/grpc")
            .body(body(b""))
            .unwrap();
        let error = RequestCodec::decode(&codec, request).await.unwrap_err();
        assert!(matches!(error, FusenError::UnsupportedProtocol(_)));
    }
}
