use std::sync::Arc;
use crate::{
    filter::{Filter, KrpcFilter, KrpcRouter},
    support::{
        triple::{TripleExceptionWrapper, TripleRequestWrapper, TripleResponseWrapper},
        TokioExecutor, TokioIo,
    },
};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{server::conn::http2, Request, Response};
use krpc_common::{KrpcMsg, RpcError};
use prost::Message;
use tracing::debug;
use super::StreamHandler;



impl StreamHandler {
    pub async fn run(mut self) {
        let mut filter_list = self.filter_list;
        filter_list.push(Filter::new(self.rpc_server));
        let server = KrpcRouter::new(
            |req: Request<hyper::body::Incoming>, filter_list: Arc<Vec<Filter>>| async move {
                let mut msg = decode_filter(req).await;
                for idx in 0..filter_list.len() {
                    msg = filter_list[idx].call(msg).await.unwrap()
                }
                return encode_filter(msg).await;
            },
            filter_list,
        );
        let hyper_io = TokioIo::new(self.tcp_stream);
        let future = http2::Builder::new(TokioExecutor)
            .adaptive_window(true)
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
    let url = req.uri().path().to_string();
    let data: Bytes = req
        .body_mut()
        .frame()
        .await
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    let trip = TripleRequestWrapper::decode(&data[5..]).unwrap();
    let path: Vec<&str> = url.split("/").collect();
    return KrpcMsg::new(
        "unique_identifier".to_string(),
        "1.0.0".to_string(),
        path[1].to_string(),
        path[2].to_string(),
        trip.get_req(),
        Result::Err(RpcError::Server("empty".to_string())),
    );
}
async fn encode_filter(
    msg: KrpcMsg,
) -> Result<Response<Full<bytes::Bytes>>, std::convert::Infallible> {
    let res_data = match msg.res {
        Ok(data) => TripleResponseWrapper::get_buf(data),
        Err(err) => TripleExceptionWrapper::get_buf(err.to_string())
    };
    let body = Full::<bytes::Bytes>::from(res_data);
    let response: Response<Full<Bytes>> = Response::builder()
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .header("grpc-status", "0")
        .body(body)
        .unwrap();
    return Ok(response);
}
