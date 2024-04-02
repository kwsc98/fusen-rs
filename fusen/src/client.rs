use crate::register::{RegisterBuilder, Resource, ResourceInfo};
use crate::route::client::Route;
use crate::support::triple::{TripleExceptionWrapper, TripleRequestWrapper, TripleResponseWrapper};
use bytes::{BufMut, BytesMut};
use fusen_common::error::FusenError;
use fusen_common::url::UrlConfig;
use fusen_common::FusenContext;
use http::{HeaderValue, Request};
use http_body_util::Full;
use hyper::client::conn::http2::SendRequest;
use prost::Message;
use rand::prelude::SliceRandom;
use serde::{Deserialize, Serialize};

pub struct FusenClient {
    route: Route,
}

impl FusenClient {
    pub fn build(register_config: Box<dyn UrlConfig>) -> FusenClient {
        let registry_builder = RegisterBuilder::new(register_config).unwrap();
        let register = registry_builder.init();
        FusenClient {
            route: Route::new(register),
        }
    }

    pub async fn invoke<Res>(&self, msg: FusenContext) -> Result<Res, FusenError>
    where
        Res: Send + Sync + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let resource_info: ResourceInfo = self
            .route
            .get_server_resource(msg.class_name.as_ref(), msg.version.as_deref())
            .await
            .map_err(|e| FusenError::Client(e.to_string()))?;
        let ResourceInfo { server_type, info } = resource_info;
        let socket_info = info
            .choose(&mut rand::thread_rng())
            .ok_or(FusenError::Client("not find server".into()))?;
        
        let sender = socket_info.socket;

        

        let mut sender: SendRequest<Full<bytes::Bytes>> = self
            .route
            .get_socket_sender(msg.class_name.as_ref(), msg.version.as_deref())
            .await
            .map_err(|e| FusenError::Client(e.to_string()))?;
        let buf = TripleRequestWrapper::get_buf(msg.req);
        let mut builder = Request::builder()
            .uri("/".to_owned() + msg.class_name.as_ref() + "/" + &msg.method_name)
            .header("content-type", "application/grpc+proto");
        if let Some(version) = msg.version {
            builder.headers_mut().unwrap().insert(
                "tri-service-version",
                HeaderValue::from_str(&version).unwrap(),
            );
        }
        let req = builder
            .body(Full::<bytes::Bytes>::from(buf))
            .map_err(|e| FusenError::Client(e.to_string()))?;
        let mut response = sender
            .send_request(req)
            .await
            .map_err(|e| FusenError::Client(e.to_string()))?;
        let mut res_body = BytesMut::new();
        loop {
            let res_frame = response
                .frame()
                .await
                .map_or(Err(FusenError::Server("error frame 1".to_owned())), |e| {
                    Ok(e)
                })?
                .map_err(|e| FusenError::Client(e.to_string()))?;
            if res_frame.is_trailers() {
                let trailers = res_frame
                    .trailers_ref()
                    .map_or(Err(FusenError::Server("error frame 2".to_owned())), |e| {
                        Ok(e)
                    })?;
                match trailers.get("grpc-status") {
                    Some(status) => match status.as_bytes() {
                        b"0" => {
                            let trip_res = TripleResponseWrapper::decode(&res_body.to_vec()[5..])
                                .map_err(|e| FusenError::Client(e.to_string()))?;
                            if trip_res.is_empty_body() {
                                return Err(FusenError::Null);
                            }
                            let res: Res = serde_json::from_slice(&trip_res.data)
                                .map_err(|e| FusenError::Client(e.to_string()))?;
                            return Ok(res);
                        }
                        else_status => {
                            if !res_body.is_empty() {
                                let trip_res: TripleExceptionWrapper =
                                    TripleExceptionWrapper::decode(&res_body.to_vec()[5..])
                                        .map_err(|e| FusenError::Client(e.to_string()))?;
                                let msg = String::from_utf8(trip_res.data).unwrap();
                                match else_status {
                                    b"90" => return Err(FusenError::Client(msg)),
                                    b"91" => return Err(FusenError::Method(msg)),
                                    b"92" => return Err(FusenError::Null),
                                    b"93" => return Err(FusenError::ResourceEmpty(msg)),
                                    _ => return Err(FusenError::Server(msg)),
                                }
                            }
                            return Err(FusenError::Server(match trailers.get("grpc-message") {
                                Some(value) => {
                                    "grpc-message=".to_owned()
                                        + &String::from_utf8(value.as_bytes().to_vec()).unwrap()
                                }
                                None => {
                                    "grpc-status=".to_owned()
                                        + &String::from_utf8(else_status.to_vec()).unwrap()
                                }
                            }));
                        }
                    },
                    None => return Err(FusenError::Server("error frame 3".to_owned())),
                }
            } else {
                let res_data = res_frame
                    .into_data()
                    .map_err(|_e| FusenError::Server("error frame 4".to_owned()))?;
                let _ = res_body.put(res_data);
            }
        }
    }
}
