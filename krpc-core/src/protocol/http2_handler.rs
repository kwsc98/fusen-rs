use std::{collections::HashMap, io::Read, sync::Arc};

use crate::{
    filter::{Filter, KrpcFilter, KrpcRouter}, protocol::compression::{decompress, CompressionEncoding}, support::{triple::TripleRequestWrapper, TokioExecutor, TokioIo}
};
use bytes::{buf::{self, Reader}, Bytes, BytesMut};
use bzip2::{read::BzDecoder, read::BzEncoder, Compression};
use http_body_util::BodyExt;
use hyper::{server::conn::http2, Request, Response};
use krpc_common::{KrpcMsg, RpcServer, RpcError};
use prost::Message;
use protobuf_json_mapping::parse_from_str;
use rand::AsByteSliceMut;
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};
use tracing::debug;

pub struct StreamHandler {
    pub tcp_stream: TcpStream,
    pub filter_list: Vec<Filter>,
    pub rpc_server: HashMap<String, Arc<Box<dyn RpcServer>>>,
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
                    msg =  filter_list[idx].call(msg).await.unwrap()
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
    println!("url : {:?}",url);
    println!("header : {:?}",req.headers());
    let mut data: Bytes  = 
        req.body_mut()
            .frame()
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap();

    // let data = "Hello, World!".as_bytes();
    // let mut de_dst = bytes::BytesMut::new();
    // decompress(CompressionEncoding::Gzip, &mut dst, &mut de_dst, len).unwrap()
    // let data = "Hello, World!".as_bytes();
    // let compressor = BzEncoder::new(data, Compression::best());
    let data = "Hello, World!".as_bytes();
let mut compressor = BzEncoder::new(data, Compression::best());
let mut dtet = String::new();
   compressor.read_to_string(&mut dtet);
    // let bug = bytes::Bytes::from(data);
    println!("de_dst : {:?}",dtet);
    let mut buf = BytesMut::default();
    let mut de = TripleRequestWrapper::default();
    de.serialize_type = "json".to_string();
    de.encode(&mut buf);
    println!("data : {:?}",buf);
    let req =  TripleRequestWrapper::decode_length_delimited(data);
    println!("data : {:?}",req);
    let path: Vec<&str> = url.split("/").collect();
    return KrpcMsg::new(
        "unique_identifier".to_string(),
        "1.0.0".to_string(),
        path[1].to_string(),
        path[2].to_string(),
        "ds".to_string(),
        Result::Err(RpcError::Server("empty".to_string()))
    );
}
async fn encode_filter(msg: KrpcMsg) -> Result<Response<String>, std::convert::Infallible> {
    let res_data= match serde_json::to_string(&msg.res) {
        Ok(data) => data,
        Err(err) => err.to_string(),
    };
    let response = Response::builder()
        .header("unique_identifier", msg.unique_identifier)
        .header("version", msg.version)
        .header("class_name", msg.class_name)
        .header("method_name", msg.method_name)
        .body(res_data)
        .unwrap();
    return Ok(response);
}
