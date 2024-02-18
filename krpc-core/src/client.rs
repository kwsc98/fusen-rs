use crate::register::RegisterBuilder;
use crate::route::Route;
use crate::support::triple::{TripleExceptionWrapper, TripleRequestWrapper, TripleResponseWrapper};
use bytes::{BufMut, BytesMut};
use http::Request;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http2::SendRequest;
use krpc_common::{KrpcMsg, RpcError};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct KrpcClient {
    route: Route,
}

impl KrpcClient {
    pub fn build(register_builder: RegisterBuilder) -> KrpcClient {
        let map = Arc::new(RwLock::new(HashMap::new()));
        let register = register_builder.init(map.clone());
        let cli = KrpcClient {
            route: Route::new(map, register),
        };
        return cli;
    }

    pub async fn invoke<Res>(&self, msg: KrpcMsg) -> Result<Res, RpcError>
    where
        Res: Send + Sync + Serialize + for<'a> Deserialize<'a> + Default,
    {
        let mut sender: SendRequest<Full<bytes::Bytes>> = self
            .route
            .get_socket_sender(&msg.class_name, &msg.version)
            .await
            .map_err(|e| RpcError::Client(e.to_string()))?;
        let buf = TripleRequestWrapper::get_buf(msg.req);
        let req = Request::builder()
            .uri("/".to_owned() + &msg.class_name + "/" + &msg.method_name)
            .header("content-type", "application/grpc+proto")
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
                match res_frame
                    .trailers_ref()
                    .map_or(Err(RpcError::Server("error frame 2".to_owned())), |e| Ok(e))?
                    .get("grpc-status")
                {
                    Some(status) => match status.as_bytes() {
                        b"0" => {
                            let trip_res = TripleResponseWrapper::decode(&res_body.to_vec()[5..])
                                .map_err(|e| RpcError::Client(e.to_string()))?;
                            if trip_res.is_empty_body() {
                                return Err(RpcError::Server("null".to_string()));
                            }
                            let res: Res = serde_json::from_slice(&trip_res.data)
                                .map_err(|e| RpcError::Client(e.to_string()))?;
                            return Ok(res);
                        }
                        _else_status => {
                            let trip_res = TripleExceptionWrapper::decode(&res_body.to_vec()[5..])
                                .map_err(|e| RpcError::Client(e.to_string()))?;
                            return Err(RpcError::Server(trip_res.get_err_info()));
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
