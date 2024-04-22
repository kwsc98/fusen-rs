use crate::codec::grpc_codec::GrpcBodyCodec;
use crate::codec::json_codec::JsonBodyCodec;
use crate::codec::request_codec::RequestHandler;
use crate::codec::BodyCodec;
use crate::register::{RegisterBuilder, Resource, ResourceInfo, SocketType};
use crate::route::client::Route;
use crate::support::triple::{TripleRequestWrapper, TripleResponseWrapper};
use crate::support::{TokioExecutor, TokioIo};
use bytes::Bytes;
use fusen_common::codec::json_field_compatible;
use fusen_common::error::FusenError;
use fusen_common::net::get_path;
use fusen_common::register::Type;
use fusen_common::url::UrlConfig;
use fusen_common::FusenContext;
use http::{request, HeaderValue, Request, Version};
use http_body_util::{BodyExt, Full};
use rand::prelude::SliceRandom;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tracing::error;
use crate::codec::request_codec::RequestCodec;

pub struct FusenClient {
    request_handle: RequestHandler<Full<Bytes>>,
    route: Route,
    json_codec: Box<
        dyn BodyCodec<bytes::Bytes, EncodeType = Vec<String>, DecodeType = Vec<String>>
            + Sync
            + Send,
    >,
    grpc_codec: Box<
        (dyn BodyCodec<
            bytes::Bytes,
            DecodeType = TripleResponseWrapper,
            EncodeType = TripleRequestWrapper,
        > + Sync
             + Send),
    >,
}

impl FusenClient {
    pub fn build(register_config: Box<dyn UrlConfig>) -> FusenClient {
        let registry_builder = RegisterBuilder::new(register_config).unwrap();
        let register = registry_builder.init();
        let json_codec = JsonBodyCodec::<bytes::Bytes>::new();
        let grpc_codec =
            GrpcBodyCodec::<bytes::Bytes, TripleResponseWrapper, TripleRequestWrapper>::new();
        FusenClient {
            request_handle: RequestHandler::<Full<Bytes>>::new(),
            route: Route::new(register),
            json_codec: Box::new(json_codec),
            grpc_codec: Box::new(grpc_codec),
        }
    }

    pub async fn invoke<Res>(
        &self,
        mut msg: FusenContext,
        return_ty: &str,
    ) -> Result<Res, FusenError>
    where
        Res: Send + Sync + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let resource_info: ResourceInfo = self
            .route
            .get_server_resource(&msg)
            .await
            .map_err(|e| FusenError::Client(e.to_string()))?;
        let ResourceInfo { server_type, info } = resource_info;
        let socket_info = info
            .choose(&mut rand::thread_rng())
            .ok_or(FusenError::Client("not find server".into()))?;
        let request = self.request_handle.encode(msg);

        let content_type = match server_type.as_ref() {
            &Type::Dubbo => ("application/grpc", "tri-service-version"),
            _ => ("application/json", "version"),
        };
        let builder = Request::builder().header("content-type", content_type.0);
        let (mut builder, body) = match path {
            fusen_common::Path::GET(path) => {
                let fields = msg.fields;
                let mut path = String::from(path);
                if fields.len() > 0 {
                    path.push_str("?");
                    for idx in 0..fields.len() {
                        path.push_str(&fields[idx]);
                        path.push_str("=");
                        path.push_str(&msg.req[idx]);
                        path.push_str("&");
                    }
                    path.remove(path.len() - 1);
                }
                (builder.method("GET").uri(path), Bytes::new())
            }
            fusen_common::Path::POST(path) => {
                let body = match server_type.as_ref() {
                    &crate::register::Type::Dubbo => {
                        let triple_request_wrapper = TripleRequestWrapper::from(msg.req);
                        self.grpc_codec
                            .encode(triple_request_wrapper)
                            .map_err(|e| FusenError::Client(e.to_string()))?
                    }
                    _ => self
                        .json_codec
                        .encode(msg.req)
                        .map_err(|e| FusenError::Client(e.to_string()))?,
                };
                (builder.method("POST").uri(path), body)
            }
        };
        if let Some(version) = msg.version {
            builder
                .headers_mut()
                .unwrap()
                .insert(content_type.1, HeaderValue::from_str(&version).unwrap());
        }
        let mut req = builder
            .header("connection", "keep-alive")
            .header("Content-Length", body.len())
            .body(Full::new(body))
            .map_err(|e| FusenError::Client(e.to_string()))?;
        let mut response = match &socket_info.socket {
            SocketType::HTTP1 => {
                *req.version_mut() = Version::HTTP_10;
                let resource = &socket_info.resource;
                let io = get_tcp_stream(&resource)
                    .await
                    .map_err(|e| FusenError::Client(e.to_string()))?;
                let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
                    .handshake(io)
                    .await
                    .map_err(|e| FusenError::Client(e.to_string()))?;
                tokio::spawn(async move {
                    if let Err(err) = conn.await {
                        error!("conn err : {}", err);
                    }
                });
                let resource = sender.send_request(req).await.map_err(|e| {
                    error!("error : {:?}", e);
                    FusenError::Client(e.to_string())
                })?;
                resource
            }
            SocketType::HTTP2(sender_lock) => {
                let sender_read = sender_lock.read().await;
                let mut sender = match sender_read.as_ref() {
                    Some(sender) => sender.clone(),
                    None => {
                        drop(sender_read);
                        let mut sender_write = sender_lock.write().await;
                        let sender = match sender_write.as_ref() {
                            Some(sender) => sender.clone(),
                            None => {
                                let resource = &socket_info.resource;
                                let io = get_tcp_stream(&resource)
                                    .await
                                    .map_err(|e| FusenError::Client(e.to_string()))?;
                                let (sender, conn) =
                                    hyper::client::conn::http2::Builder::new(TokioExecutor)
                                        .adaptive_window(true)
                                        .handshake(io)
                                        .await
                                        .map_err(|e| FusenError::Client(e.to_string()))?;
                                let sender_lock = sender_lock.clone();
                                tokio::spawn(async move {
                                    let sender = sender_lock;
                                    if let Err(err) = conn.await {
                                        sender.write().await.take();
                                        error!("conn err : {}", err);
                                    }
                                });
                                let _ = sender_write.insert(sender.clone());
                                sender
                            }
                        };
                        sender
                    }
                };
                let resource = sender.send_request(req).await.map_err(|e| {
                    error!("{:?}", e);
                    FusenError::Client(e.to_string())
                })?;
                resource
            }
        };
        if !response.status().is_success() {
            if response.status().is_client_error() {
                return Err(FusenError::Client(format!(
                    "err code : {}",
                    response.status().as_str()
                )));
            } else if response.status().is_server_error() {
                return Err(FusenError::Server(format!(
                    "err code : {}",
                    response.status().as_str()
                )));
            }
        }
        let mut frame_vec = vec![];
        while let Some(body) = response.frame().await {
            if let Ok(body) = body {
                if body.is_trailers() {
                    let trailers = body
                        .trailers_ref()
                        .map_or(Err(FusenError::Server("error frame".to_owned())), |e| Ok(e))?;
                    match trailers.get("grpc-status") {
                        Some(status) => match status.as_bytes() {
                            b"0" => {
                                break;
                            }
                            else_status => {
                                let msg = match trailers.get("grpc-message") {
                                    Some(value) => {
                                        String::from_utf8(value.as_bytes().to_vec()).unwrap()
                                    }
                                    None => {
                                        "grpc-status=".to_owned()
                                            + &String::from_utf8(else_status.to_vec()).unwrap()
                                    }
                                };
                                match else_status {
                                    b"90" => return Err(FusenError::Client(msg)),
                                    b"91" => return Err(FusenError::Method(msg)),
                                    b"92" => return Err(FusenError::Null),
                                    b"93" => return Err(FusenError::ResourceEmpty(msg)),
                                    _ => return Err(FusenError::Server(msg)),
                                };
                            }
                        },
                        None => return Err(FusenError::Server("error frame".to_owned())),
                    }
                }
                frame_vec.push(body);
            } else {
                break;
            }
        }
        let res = match server_type.as_ref() {
            &crate::register::Type::Dubbo => {
                let response = self
                    .grpc_codec
                    .decode(frame_vec)
                    .map_err(|e| FusenError::Client(e.to_string()))?;
                String::from_utf8(response.data).map_err(|e| FusenError::Client(e.to_string()))?
            }
            _ => {
                let mut response = self
                    .json_codec
                    .decode(frame_vec)
                    .map_err(|e| FusenError::Client(e.to_string()))?;
                response.remove(0)
            }
        };
        let res = json_field_compatible(return_ty, res);
        let res: Res = serde_json::from_str(&res).map_err(|e| FusenError::Client(e.to_string()))?;
        return Ok(res);
    }
}

async fn get_tcp_stream(resource: &Resource) -> Result<TokioIo<TcpStream>, crate::Error> {
    let url = get_path(resource.ip.clone(), resource.port.as_deref())
        .parse::<hyper::Uri>()
        .map_err(|e| FusenError::Client(e.to_string()))?;
    let host = url.host().expect("uri has no host");
    let port = url.port_u16().unwrap_or(80);
    let addr = format!("{}:{}", host, port);
    Ok(TcpStream::connect(addr)
        .await
        .map(TokioIo::new)
        .map_err(|e| FusenError::Client(e.to_string()))?)
}
