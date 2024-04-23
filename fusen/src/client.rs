use crate::codec::grpc_codec::GrpcBodyCodec;
use crate::codec::json_codec::JsonBodyCodec;
use crate::codec::request_codec::RequestCodec;
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
use fusen_common::register::{self, Type};
use fusen_common::url::UrlConfig;
use fusen_common::FusenContext;
use http::Version;
use http_body_util::{BodyExt, Full};
use rand::prelude::SliceRandom;
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tracing::error;

pub struct FusenClient {
    request_handle: RequestHandler,
    route: Route,
}

impl FusenClient {
    pub fn build(register_config: Box<dyn UrlConfig>) -> FusenClient {
        let registry_builder = RegisterBuilder::new(register_config).unwrap();
        let register = registry_builder.init();
        let json_codec = JsonBodyCodec::<bytes::Bytes>::new();
        let grpc_codec =
            GrpcBodyCodec::<bytes::Bytes, TripleResponseWrapper, TripleRequestWrapper>::new();
        FusenClient {
            request_handle: RequestHandler::new(),
            route: Route::new(register),
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
        let ResourceInfo { server_type, socket } = resource_info;
        let socket = socket
            .choose(&mut rand::thread_rng())
            .ok_or(FusenError::Client("not find server".into()))?;
        let mut request = self.request_handle.encode(msg)?;
        let mut response: http::Response<hyper::body::Incoming> = socket.send_request(request).await?;
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
