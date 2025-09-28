use crate::{
    error::FusenError,
    protocol::{
        codec::body::{RequestBodyCodec, ResponseBodyCodec, json::JsonCodec, triple::TripleCodec},
        fusen::{
            request::{FusenRequest, Path},
            response::{FusenResponse, HttpStatus},
        },
    },
};
use bytes::{Bytes, BytesMut};
use fusen_internal_common::protocol::Protocol;
use http::{
    Request, Response, Version,
    header::{CONNECTION, CONTENT_TYPE},
};
use http_body_util::{BodyExt, Full, combinators::BoxBody};
use std::{collections::HashMap, convert::Infallible};

pub mod body;

#[derive(Default)]
pub struct FusenHttpCodec {
    json_codec: JsonCodec,
    triple_codec: TripleCodec,
}

impl RequestCodec<Bytes, hyper::Error> for FusenHttpCodec {
    fn encode(
        &self,
        fusen_request: &mut FusenRequest,
    ) -> Result<
        Request<http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>>,
        crate::error::FusenError,
    > {
        let mut builder = Request::builder().header(CONNECTION, "keep-alive");
        for (key, value) in fusen_request.headers.drain() {
            builder = builder.header(key, value);
        }
        let Some(addr) = &fusen_request.addr else {
            return Err(FusenError::Impossible);
        };
        let mut uri = format!("{}{}", addr, fusen_request.path.path);
        if !fusen_request.querys.is_empty() {
            if fusen_request.path.path.contains('{') {
                let mut path = fusen_request.path.path.to_string();
                for (key, value) in &fusen_request.querys {
                    path = path.replace(&format!("{{{key}}}"), value);
                }
                uri = format!("{addr}{path}");
            } else {
                uri.push('?');
                for (key, value) in &fusen_request.querys {
                    uri.push_str(&format!("{}={}&", key, urlencoding::encode(value.as_str())));
                }
                uri.pop();
            }
        }
        let mut body = Bytes::new();
        let mut version = Version::HTTP_2;
        if let Some(bodys) = fusen_request.bodys.take() {
            match &fusen_request.protocol {
                Protocol::Dubbo => {
                    builder = builder.header(CONTENT_TYPE, "application/grpc");
                    body = RequestBodyCodec::encode(&self.triple_codec, bodys)?;
                }
                _ => {
                    if let Protocol::Host(_) | Protocol::SpringCloud(_) = fusen_request.protocol {
                        version = Version::HTTP_11;
                    }
                    builder = builder.header(CONTENT_TYPE, "application/json");
                    body = RequestBodyCodec::encode(&self.json_codec, bodys)?;
                }
            };
        }
        builder
            .version(version)
            .method(fusen_request.path.method.clone())
            .uri(uri)
            .body(Full::new(body).boxed())
            .map_err(|error| crate::error::FusenError::Error(Box::new(error)))
    }

    async fn decode(
        &self,
        mut request: Request<http_body_util::combinators::BoxBody<Bytes, hyper::Error>>,
    ) -> Result<FusenRequest, crate::error::FusenError> {
        let mut querys = HashMap::new();
        let mut headers = HashMap::new();
        for (key, value) in request.headers_mut().drain() {
            let Some(key) = key else {
                continue;
            };
            headers.insert(
                key.to_string().to_ascii_lowercase(),
                String::from_utf8_lossy(value.as_bytes()).to_string(),
            );
        }
        if let Some(request_querys) = request.uri().query() {
            let request_querys: Vec<&str> = request_querys.split('&').collect();
            for query in request_querys {
                if let Some((key, value)) = query.split_once('=') {
                    querys.insert(
                        key.to_string(),
                        urlencoding::decode(value)
                            .map(|e| e.to_string())
                            .unwrap_or(value.to_string()),
                    );
                }
            }
        }
        let mut protocol = Protocol::Fusen;
        let mut bodys = None;
        if let Some(content_type) = headers.get(CONTENT_TYPE.as_str()) {
            let bytes = read_body(request.body_mut()).await.freeze();
            if content_type.starts_with("application/grpc") {
                protocol = Protocol::Dubbo;
                let _ = bodys.insert(RequestBodyCodec::decode(&self.triple_codec, bytes)?);
            } else if content_type.starts_with("application/json") {
                let _ = bodys.insert(RequestBodyCodec::decode(&self.json_codec, bytes)?);
            }
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
            protocol,
        })
    }
}

impl ResponseCodec<Bytes, hyper::Error> for FusenHttpCodec {
    fn encode(
        &self,
        fusen_response: &mut FusenResponse,
    ) -> Result<
        http::Response<http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>>,
        crate::error::FusenError,
    > {
        let mut builder = Response::builder();
        for (key, value) in fusen_response.headers.drain() {
            builder = builder.header(key, value);
        }
        let mut body = Bytes::new();
        if let Some(bodys) = fusen_response.body.take() {
            match &fusen_response.protocol {
                Protocol::Dubbo => {
                    builder = builder.header(CONTENT_TYPE, "application/grpc");
                    body = ResponseBodyCodec::encode(&self.triple_codec, bodys)?;
                }
                _ => {
                    builder = builder.header(CONTENT_TYPE, "application/json");
                    body = ResponseBodyCodec::encode(&self.json_codec, bodys)?;
                }
            };
        }
        builder
            .status(fusen_response.http_status.status)
            .body(Full::new(body).boxed())
            .map_err(|error| FusenError::Error(Box::new(error)))
    }

    async fn decode(
        &self,
        mut response: http::Response<http_body_util::combinators::BoxBody<Bytes, hyper::Error>>,
    ) -> Result<FusenResponse, crate::error::FusenError> {
        let mut headers: HashMap<String, String> = HashMap::new();
        for (key, value) in response.headers_mut().drain() {
            let Some(key) = key else {
                continue;
            };
            headers.insert(
                key.to_string().to_ascii_lowercase(),
                String::from_utf8_lossy(value.as_bytes()).to_string(),
            );
        }
        let mut protocol = Protocol::Fusen;
        let mut body = None;
        if let Some(content_type) = headers.get(CONTENT_TYPE.as_str()) {
            let bytes = read_body(response.body_mut()).await.freeze();
            if content_type.starts_with("application/grpc") {
                protocol = Protocol::Dubbo;
                let _ = body.insert(ResponseBodyCodec::decode(&self.triple_codec, bytes)?);
            } else if content_type.starts_with("application/json") {
                let _ = body.insert(ResponseBodyCodec::decode(&self.json_codec, bytes)?);
            }
        };
        Ok(FusenResponse {
            protocol,
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

#[allow(async_fn_in_trait)]
pub trait RequestCodec<T, E> {
    fn encode(
        &self,
        fusen_request: &mut FusenRequest,
    ) -> Result<Request<BoxBody<T, Infallible>>, FusenError>;

    async fn decode(&self, request: Request<BoxBody<T, E>>) -> Result<FusenRequest, FusenError>;
}

#[allow(async_fn_in_trait)]
pub trait ResponseCodec<T, E> {
    fn encode(
        &self,
        fusen_response: &mut FusenResponse,
    ) -> Result<Response<BoxBody<T, Infallible>>, FusenError>;

    async fn decode(&self, response: Response<BoxBody<T, E>>) -> Result<FusenResponse, FusenError>;
}

async fn read_body(body: &mut BoxBody<Bytes, hyper::Error>) -> BytesMut {
    let mut mut_bytes = BytesMut::new();
    while let Some(Ok(frame)) = body.frame().await {
        if let Ok(bytes) = frame.into_data() {
            mut_bytes.extend_from_slice(&bytes);
        }
    }
    mut_bytes
}
