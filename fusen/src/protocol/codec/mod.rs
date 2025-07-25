use crate::{
    error::FusenError,
    protocol::{
        codec::body::{RequestBodyCodec, json::JsonCodec, triple::TripleCodec},
        fusen::{
            context::FusenContext,
            request::{FusenRequest, Path},
            response::FusenResponse,
        },
    },
};
use bytes::{Bytes, BytesMut};
use http::{
    Method, Request, Response,
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
        context: &mut FusenContext,
    ) -> Result<
        Request<http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>>,
        crate::error::FusenError,
    > {
        let fusen_request = &mut context.request;
        let mut builder = Request::builder().header(CONNECTION, "keep-alive");
        for (key, value) in fusen_request.headers.drain() {
            builder = builder.header(key, value);
        }
        let mut uri = fusen_request.path.uri.path().to_owned();
        if !fusen_request.querys.is_empty() {
            uri.push_str("?");
            for (key, value) in &fusen_request.querys {
                uri.push_str(&format!("{}={}&", key, urlencoding::encode(value.as_str())));
            }
            uri.pop();
        }
        let mut body = Bytes::new();
        if let Method::POST = fusen_request.path.method {
            match &context.protocol {
                super::Protocol::Dubbo => {
                    builder = builder.header(CONTENT_TYPE, "application/grpc");
                    body = self
                        .triple_codec
                        .encode(fusen_request.body.drain(..).collect())?;
                }
                _ => {
                    builder = builder.header(CONTENT_TYPE, "application/json");
                    body = self
                        .triple_codec
                        .encode(fusen_request.body.drain(..).collect())?;
                }
            };
        }
        builder
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
        Ok(FusenRequest {
            path: Path {
                method: request.method().clone(),
                uri: request.uri().clone(),
            },
            querys,
            headers,
            extensions: None,
            body: (),
        })
    }
}

impl ResponseCodec<Bytes, hyper::Error> for FusenHttpCodec {
    fn encode(
        &self,
        context: &mut FusenContext,
    ) -> Result<
        http::Response<http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>>,
        crate::error::FusenError,
    > {
        todo!()
    }

    async fn decode(
        &self,
        request: http::Response<http_body_util::combinators::BoxBody<Bytes, hyper::Error>>,
    ) -> Result<FusenResponse, crate::error::FusenError> {
        todo!()
    }
}

pub trait RequestCodec<T, E> {
    fn encode(
        &self,
        context: &mut FusenContext,
    ) -> Result<Request<BoxBody<T, Infallible>>, FusenError>;

    async fn decode(&self, request: Request<BoxBody<T, E>>) -> Result<FusenRequest, FusenError>;
}

pub trait ResponseCodec<T, E> {
    fn encode(
        &self,
        context: &mut FusenContext,
    ) -> Result<Response<BoxBody<T, Infallible>>, FusenError>;

    async fn decode(&self, request: Response<BoxBody<T, E>>) -> Result<FusenResponse, FusenError>;
}
