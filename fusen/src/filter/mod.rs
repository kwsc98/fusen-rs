use fusen_common::{FusenError, FusenFuture, FusenMsg, RpcServer};
use futures::Future;
use http::{HeaderMap, HeaderValue};
use http_body::Frame;
use http_body_util::{BodyExt, StreamBody};
use hyper::{service::Service, Request, Response};
use prost::Message;
use std::{collections::HashMap, convert::Infallible, sync::Arc};

use crate::support::triple::{TripleExceptionWrapper, TripleRequestWrapper, TripleResponseWrapper};
type TrailersBody = futures_util::stream::Iter<
    std::vec::IntoIter<std::result::Result<http_body::Frame<bytes::Bytes>, Infallible>>,
>;
pub struct FusenRouter<KF: 'static> {
    fusen_filter: &'static KF,
}

impl<KF> FusenRouter<KF>
where
    KF: FusenFilter<Request = FusenMsg, Response = FusenMsg, Error = crate::Error> + Clone,
{
    pub fn new(fusen_filter: &'static KF) -> Self {
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
        eprintln!("{:?}", req);
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
                if data.starts_with(b"[") {
                    match serde_json::from_slice(&data) {
                        Ok(req) => req,
                        Err(err) => return Err(FusenError::Client(err.to_string())),
                    }
                } else {
                    vec![String::from_utf8(data.to_vec()).unwrap()]
                }
            };
            let url = req.uri().path().to_string();
            let version = req
                .headers()
                .get("tri-service-version")
                .map(|e| String::from_utf8_lossy(e.as_bytes()).to_string());
            let msg = FusenMsg::new_server("unique_identifier".to_string(), version, url, req_body);
            let msg = match fusen_filter.call(msg).await {
                Ok(msg) => msg,
                Err(err) => return Err(FusenError::Server(err.to_string())),
            };
            eprintln!("{:?}", msg);
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
    cache: HashMap<String, Arc<Box<dyn RpcServer>>>,
    path_cache: HashMap<String, (String, String)>,
}

impl RpcServerRoute {
    pub fn new(cache: HashMap<String, Arc<Box<dyn RpcServer>>>) -> Self {
        let mut path_cache = HashMap::new();
        for item in &cache {
            let (id, version, methods) = item.1.get_info();
            for method in methods {
                let method_info = method.into();
                let mut path_rpc = "/".to_owned() + id + "/" + &method_info.0;
                let mut path = method_info.1;
                if let Some(version) = version {
                    path_rpc.push_str("?version=");
                    path_rpc.push_str(version);

                    path.push_str("?version=");
                    path.push_str(version);
                }
                path_cache.insert(path_rpc, (id.to_string(), method_info.2.clone()));
                path_cache.insert(path, (id.to_string(), method_info.2));
            }
        }
        return RpcServerRoute { cache, path_cache };
    }
    pub fn get_server(&self, msg: &mut FusenMsg) -> Option<Arc<Box<dyn RpcServer>>> {
        let info = self.path_cache.get(&msg.path)?;
        msg.class_name = info.0.clone();
        msg.method_name = info.1.clone();
        let mut class_name = msg.class_name.clone();
        if let Some(version) = &msg.version {
            class_name.push_str(":");
            class_name.push_str(version);
        }
        self.cache.get(&class_name).map(|e| e.clone())
    }
}

impl FusenFilter for RpcServerRoute {
    type Request = FusenMsg;

    type Response = FusenMsg;

    type Error = crate::Error;

    type Future = crate::FusenFuture<Result<Self::Response, Self::Error>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let mut msg: FusenMsg = req;
        let server = self.get_server(&mut msg);
        match server {
            Some(server) => Box::pin(async move { Ok(server.invoke(msg).await) }),
            None => Box::pin(async move {
                msg.res = Err(FusenError::Server(format!(
                    "not find server by {:?} version {:?}",
                    msg.class_name, msg.version
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
