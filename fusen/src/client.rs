use crate::register::RegisterBuilder;
use crate::route::Route;
use crate::support::triple::{TripleExceptionWrapper, TripleRequestWrapper, TripleResponseWrapper};
use bytes::{BufMut, BytesMut};
use fusen_common::{FusenMsg, RpcError};
use http::{HeaderValue, Request};
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http2::SendRequest;
use prost::Message;
use serde::{Deserialize, Serialize};

pub struct FusenClient {
    route: Route,
}

impl FusenClient {
    pub fn build(register_builder: RegisterBuilder) -> FusenClient {
        let register = register_builder.init();
        let cli = FusenClient {
            route: Route::new(register),
        };
        return cli;
    }

    pub async fn invoke<Res>(&self, msg: FusenMsg) -> Result<Res, RpcError>
    where
        Res: Send + Sync + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let mut sender: SendRequest<Full<bytes::Bytes>> = self
            .route
            .get_socket_sender(&msg.class_name, msg.version.as_deref())
            .await
            .map_err(|e| RpcError::Client(e.to_string()))?;
        let buf = TripleRequestWrapper::get_buf(msg.req);
        let mut builder = Request::builder()
            .uri("/".to_owned() + &msg.class_name + "/" + &msg.method_name)
            .header("content-type", "application/grpc+proto");
        if let Some(version) = msg.version {
            builder.headers_mut().unwrap().insert(
                "tri-service-version",
                HeaderValue::from_str(&version).unwrap(),
            );
        }
        let req = builder
            .body(Full::<bytes::Bytes>::from(buf))
            .map_err(|e| RpcError::Client(e.to_string()))?;
        let mut response = sender
            .send_request(req)
            .await
            .map_err(|e| RpcError::Client(e.to_string()))?;
        let mut res_body = BytesMut::new();
        loop {
            let res_frame = response
                .frame()
                .await
                .map_or(Err(RpcError::Server("error frame 1".to_owned())), |e| Ok(e))?
                .map_err(|e| RpcError::Client(e.to_string()))?;
            if res_frame.is_trailers() {
                let trailers = res_frame
                    .trailers_ref()
                    .map_or(Err(RpcError::Server("error frame 2".to_owned())), |e| Ok(e))?;
                match trailers.get("grpc-status") {
                    Some(status) => match status.as_bytes() {
                        b"0" => {
                            let trip_res = TripleResponseWrapper::decode(&res_body.to_vec()[5..])
                                .map_err(|e| RpcError::Client(e.to_string()))?;
                            if trip_res.is_empty_body() {
                                return Err(RpcError::Null);
                            }
                            let res: Res = serde_json::from_slice(&trip_res.data)
                                .map_err(|e| RpcError::Client(e.to_string()))?;
                            return Ok(res);
                        }
                        else_status => {
                            if !res_body.is_empty() {
                                let trip_res: TripleExceptionWrapper =
                                    TripleExceptionWrapper::decode(&res_body.to_vec()[5..])
                                        .map_err(|e| RpcError::Client(e.to_string()))?;
                                let msg = String::from_utf8(trip_res.data).unwrap();
                                match else_status {
                                    b"90" => return Err(RpcError::Client(msg)),
                                    b"91" => return Err(RpcError::Method(msg)),
                                    b"92" => return Err(RpcError::Null),
                                    _ => return Err(RpcError::Server(msg)),
                                }
                            }
                            return Err(RpcError::Server(match trailers.get("grpc-message") {
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
                    None => return Err(RpcError::Server("error frame 3".to_owned())),
                }
            } else {
                let res_data = res_frame
                    .into_data()
                    .map_err(|_e| RpcError::Server("error frame 4".to_owned()))?;
                let _ = res_body.put(res_data);
            }
        }
    }
}
