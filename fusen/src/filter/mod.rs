use bytes::Bytes;
use fusen_common::{FusenError, FusenFuture, FusenMsg, RpcServer};
use futures::Future;
use http::{HeaderMap, HeaderValue};
use http_body::{Body, Frame};
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::{service::Service, Request, Response};
use prost::Message;
use std::{collections::HashMap, convert::Infallible, sync::Arc};

use crate::support::triple::{TripleExceptionWrapper, TripleRequestWrapper, TripleResponseWrapper};
type TrailersBody = futures_util::stream::Iter<
    std::vec::IntoIter<std::result::Result<http_body::Frame<bytes::Bytes>, Infallible>>,
>;
pub struct FusenRouter<KF> {
    fusen_filter: KF,
}

impl<KF> FusenRouter<KF>
where
    KF: FusenFilter<Request = FusenMsg, Response = FusenMsg, Error = crate::Error> + Clone,
{
    pub fn new(fusen_filter: KF) -> Self {
        return FusenRouter { fusen_filter };
    }
}

impl<KF> Service<Request<hyper::body::Incoming>> for FusenRouter<KF>
where
    KF: FusenFilter<Request = FusenMsg, Response = FusenMsg, Error = crate::Error>
        + Clone
        + Send
        + 'static,
{
    type Response = Response<StreamBody<TrailersBody>>;
    type Error = FusenError;
    type Future = FusenFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, mut req: Request<hyper::body::Incoming>) -> Self::Future {
        let fusen_filter = self.fusen_filter.clone();
        Box::pin(async move {
            let content_type = req
                .headers()
                .get("content-type")
                .map_or("application/json", |e| e.to_str().unwrap())
                .to_string();
            let mut vec = vec![];
            while let Some(frame) = req.frame().await {
                match frame {
                    Ok(frame) => {
                        if frame.is_data() {
                            vec.push(frame.into_data().unwrap());
                        } else {
                            break;
                        }
                    }
                    Err(err) => return Err(FusenError::Client(err.to_string())),
                }
            }
            let data = if vec.is_empty() {
                return Err(FusenError::Client("err req".to_string()));
            } else {
                &vec[0]
            };
            let req_body: Vec<String> = if content_type.contains("grpc") {
                match TripleRequestWrapper::decode(&data[5..]) {
                    Ok(req) => req.get_req(),
                    Err(err) => {
                        return Err(FusenError::Client(err.to_string()));
                    }
                }
            } else {
                match serde_json::from_slice(&data) {
                    Ok(req) => req,
                    Err(err) => return Err(FusenError::Client(err.to_string())),
                }
            };
            let url = req.uri().path().to_string();
            let path: Vec<&str> = url.split("/").collect();
            let version = req
                .headers()
                .get("tri-service-version")
                .map(|e| String::from_utf8_lossy(e.as_bytes()).to_string());
            let msg = FusenMsg::new(
                "unique_identifier".to_string(),
                version,
                path[1].to_string(),
                path[2].to_string(),
                req_body,
                Result::Err(FusenError::Server("empty".to_string())),
            );
            let msg = match fusen_filter.call(msg).await {
                Ok(msg) => msg,
                Err(err) => return Err(FusenError::Server(err.to_string())),
            };
            let response = Response::builder();
            if content_type.contains("grpc") {
                let mut status = "0";
                let res_data = match msg.res {
                    Ok(data) => bytes::Bytes::from(TripleResponseWrapper::get_buf(data)),
                    Err(err) => bytes::Bytes::from(TripleExceptionWrapper::get_buf(match err {
                        FusenError::Client(msg) => {
                            status = "90";
                            msg
                        }
                        FusenError::Method(msg) => {
                            status = "91";
                            msg
                        }
                        FusenError::Null => {
                            status = "92";
                            "FusenError::Null".to_string()
                        }
                        FusenError::Server(msg) => {
                            status = "93";
                            msg
                        }
                    })),
                };
                let mut trailers = HeaderMap::new();
                trailers.insert("grpc-status", HeaderValue::from_str(&status).unwrap());
                let chunks: Vec<Result<_, Infallible>> =
                    vec![Ok(Frame::data(res_data)), Ok(Frame::trailers(trailers))];
                let stream = futures_util::stream::iter(chunks);
                let stream_body = http_body_util::StreamBody::new(stream);
                response
                    .header("content-type", "application/grpc")
                    .header("te", "trailers")
                    .body(stream_body)
                    .map_err(|e| FusenError::Server(e.to_string()))
            } else {
                match msg.res {
                    Ok(data) => {
                        let chunks: Vec<Result<_, Infallible>> = vec![Ok(Frame::data(data.into()))];
                        let stream = futures_util::stream::iter(chunks);
                        let stream_body = http_body_util::StreamBody::new(stream);
                        response
                            .body(stream_body)
                            .map_err(|e| FusenError::Server(e.to_string()))
                    }
                    Err(_err) => {
                        let mut trailers = HeaderMap::new();
                        trailers.insert("grpc-status", HeaderValue::from_str("93").unwrap());
                        let chunks: Vec<Result<_, Infallible>> =
                            vec![Ok(Frame::trailers(trailers))];
                        let stream = futures_util::stream::iter(chunks);
                        let stream_body = http_body_util::StreamBody::new(stream);
                        response
                            .status(504)
                            .body(stream_body)
                            .map_err(|e| FusenError::Server(e.to_string()))
                    }
                }
            }
        })
    }
}

#[derive(Clone, Default)]
pub struct RpcServerRoute {
    map: HashMap<String, Arc<Box<dyn RpcServer>>>,
}

impl RpcServerRoute {
    pub fn new(map: HashMap<String, Arc<Box<dyn RpcServer>>>) -> Self {
        return RpcServerRoute { map };
    }
}

impl FusenFilter for RpcServerRoute {
    type Request = FusenMsg;

    type Response = FusenMsg;

    type Error = crate::Error;

    type Future = crate::FusenFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let mut msg: FusenMsg = req;
        let mut class_name = msg.class_name.clone();
        if let Some(version) = &msg.version {
            class_name.push_str(":");
            class_name.push_str(version);
        }
        match self.map.get(&class_name) {
            Some(server) => {
                let server = server.clone();
                Box::pin(async move { Ok(server.invoke(msg).await) })
            }
            None => Box::pin(async move {
                msg.res = Err(FusenError::Server(format!(
                    "not find server by {}",
                    class_name
                )));
                Ok(msg)
            }),
        }
    }
}

pub trait FusenFilter {
    type Request;

    type Response: Send;

    type Error: Send;

    type Future: Future<Output = Result<Self::Response, Self::Error>> + Send;

    fn call(&self, req: Self::Request) -> Self::Future;
}
