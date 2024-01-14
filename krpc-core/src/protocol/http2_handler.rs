use std::{collections::HashMap, sync::Arc};

use crate::{
    filter::{Filter, KrpcFilter, KrpcRouter},
    support::{TokioExecutor, TokioIo},
};
use http_body_util::BodyExt;
use hyper::{server::conn::http2, Request, Response};
use krpc_common::{KrpcMsg, RpcServer};
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc, Mutex},
};
use tracing::debug;

pub struct StreamHandler {
    pub tcp_stream: TcpStream,
    pub filter_list: Vec<Filter>,
    pub rpc_server: HashMap<String, Arc<Mutex<Box<dyn RpcServer>>>>,
    pub shutdown: broadcast::Receiver<()>,
    pub _shutdown_complete: mpsc::Sender<()>,
}

impl StreamHandler {
    pub async fn run(mut self) {
        let mut filter_list = self.filter_list;
        filter_list.push(Filter::new(self.rpc_server));
        let server = KrpcRouter::new(
            |req: Request<hyper::body::Incoming>, filter_list: Arc<Vec<Filter>>| async move {
                let mut msg = decode_filter(req).await;
                for idx in 0..filter_list.len() {
                    msg = filter_list[idx].call(msg).await.unwrap();
                }
                return encode_filter(msg).await;
            },
            filter_list,
        );
        let hyper_io = TokioIo::new(self.tcp_stream);
        let future = http2::Builder::new(TokioExecutor)
            .initial_stream_window_size(100000)
            .initial_connection_window_size(100000)
            .adaptive_window(false)
            .serve_connection(hyper_io, server);
        let err_info = tokio::select! {
                res = future =>
                    match res {
                        Ok(_) => "client close".to_string(),
                        Err(err) => err.to_string(),
                    }
                 ,
                res2 = self.shutdown.recv() => match res2 {
                    Ok(_) => "shutdown error".to_string(),
                    Err(_) => "server shutdown".to_string(),
                }
        };
        debug!("connect close by {}", err_info);
    }
}

async fn decode_filter(mut req: Request<hyper::body::Incoming>) -> KrpcMsg {
    let unique_identifier = req
        .headers()
        .get("unique_identifier")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let version = req
        .headers()
        .get("version")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let class_name = req
        .headers()
        .get("class_name")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let method_name = req
        .headers()
        .get("method_name")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let data = String::from_utf8(
        req.body_mut()
            .frame()
            .await
            .unwrap()
            .unwrap()
            .data_ref()
            .unwrap()
            .as_ref()
            .to_vec(),
    );
    return KrpcMsg::new(
        unique_identifier,
        version,
        class_name,
        method_name,
        data.unwrap(),
    );
}
async fn encode_filter(msg: KrpcMsg) -> Result<Response<String>, std::convert::Infallible> {
    let response = Response::builder()
        .header("unique_identifier", msg.unique_identifier)
        .header("version", msg.version)
        .header("class_name", msg.class_name)
        .header("method_name", msg.method_name)
        .body(msg.data)
        .unwrap();
    return Ok(response);
}
