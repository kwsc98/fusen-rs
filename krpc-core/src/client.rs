use crate::register::RegisterBuilder;
use crate::route::Route;
use crate::support::triple::{TripleRequestWrapper, TripleResponseWrapper};
use bytes::BufMut;
use http::Request;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http2::SendRequest;
use krpc_common::{KrpcMsg, RpcError};
use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
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
        let mut res = sender
            .send_request(req)
            .await
            .map_err(|e| RpcError::Client(e.to_string()))?;
        println!("header {:?}",res.headers());
        let res_data = res.frame()
        .await
        .unwrap()
        .map_err(|e| RpcError::Client(e.to_string()))?
        .into_data().unwrap();
        println!("data {:?}",res_data);

        let trip_res = TripleResponseWrapper::decode(&res_data.to_vec()[5..]).map_err(|e|RpcError::Client(e.to_string()))?;
        let res: Res = serde_json::from_slice(
            &trip_res.data,
        )
        .map_err(|e| RpcError::Client(e.to_string()))?;
        return Ok(res);
    }
}
