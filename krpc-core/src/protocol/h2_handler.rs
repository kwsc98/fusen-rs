use std::{collections::HashMap, io::Read, sync::Arc, time::Duration};

use super::StreamHandler;
use crate::{
    filter::{Filter, KrpcFilter, KrpcRouter},
    protocol::compression::{decompress, CompressionEncoding},
    support::{
        triple::{TripleExceptionWrapper, TripleRequestWrapper, TripleResponseWrapper},
        TokioExecutor, TokioIo,
    },
};
use bytes::{
    buf::{self, Reader},
    Buf, BufMut, Bytes, BytesMut,
};
use h2::server::{self, Builder};
use http::{HeaderMap, HeaderValue, Request, Response};
use http_body::Body;
use http_body_util::{BodyExt, Full};
use krpc_common::{KrpcMsg, RpcError, RpcServer};
use prost::Message;
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};

impl StreamHandler {
    pub async fn run_v2(mut self) {
        let mut connection = get_server_builder()
            .handshake::<_, Bytes>(self.tcp_stream)
            .await
            .unwrap();
        // let mut mpsc: (mpsc::Sender<i32>, mpsc::Receiver<i32>) = mpsc::channel(1);
        self.filter_list.push(Filter::new(self.rpc_server));
        while let Some(result) = connection.accept().await {
            // if let Err(err) = self.shutdown.try_recv() {
            //     if let tokio::sync::broadcast::error::TryRecvError::Closed = err {
            //         mpsc.1.recv().await;
            //         return;
            //     }
            // }
            // let send_mpsc = mpsc.0.clone();
            let filter_list = self.filter_list.clone();
            tokio::spawn(async move {
                let (request, mut respond) = result.unwrap();
                let mut msg = decode_filter(request).await;
                for idx in 0..filter_list.len() {
                    msg = filter_list[idx].call(msg).await.unwrap()
                }
                let res = encode_filter(msg).await;
                let mut send = respond.send_response(res.0, false).unwrap();
                let _ = send.send_data(res.1, false);
                // let mut trailers = HeaderMap::new();
                // trailers.append("grpc-status", HeaderValue::from(0));
                // let _ = send.send_trailers(trailers);
                // drop(send_mpsc);
            });
        }
    }
}

async fn decode_filter(mut req: Request<h2::RecvStream>) -> KrpcMsg {
    let url = req.uri().path().to_string();
    let data = req.body_mut().data().await.unwrap().unwrap();
    let trip = match TripleRequestWrapper::decode(&data[5..]) {
        Ok(data) => data,
        Err(err) => {
            println!("{:?}" ,err);
            println!("{:?}" ,data.to_vec());
            println!("{:?}" ,req);
            panic!();
        },
    }; 
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
async fn encode_filter(msg: KrpcMsg) -> (Response<()>, bytes::Bytes) {
    let res_data = match msg.res {
        Ok(data) => TripleResponseWrapper::get_buf(data),
        Err(err) => TripleExceptionWrapper::get_buf(err.to_string()),
    };
    let body = bytes::Bytes::from(res_data);
    let response: Response<()> = Response::builder()
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(())
        .unwrap();
    return (response, body);
}



fn get_server_builder() -> Builder {
    let mut config : Config= Default::default();
    config.initial_conn_window_size = SPEC_WINDOW_SIZE;
    config.initial_stream_window_size = SPEC_WINDOW_SIZE;
    let mut builder = h2::server::Builder::default();
    builder
        .initial_window_size(config.initial_stream_window_size)
        .initial_connection_window_size(config.initial_conn_window_size)
        .max_frame_size(config.max_frame_size)
        .max_header_list_size(config.max_header_list_size)
        .max_send_buffer_size(config.max_send_buffer_size);
    if let Some(max) = config.max_concurrent_streams {
        builder.max_concurrent_streams(max);
    }
    if config.enable_connect_protocol {
        builder.enable_connect_protocol();
    }
    return builder;
}




// Our defaults are chosen for the "majority" case, which usually are not
// resource constrained, and so the spec default of 64kb can be too limiting
// for performance.
//
// At the same time, a server more often has multiple clients connected, and
// so is more likely to use more resources than a client would.
const DEFAULT_CONN_WINDOW: u32 = 1024 * 1024; // 1mb
const DEFAULT_STREAM_WINDOW: u32 = 1024 * 1024; // 1mb
const DEFAULT_MAX_FRAME_SIZE: u32 = 1024 * 16; // 16kb
const DEFAULT_MAX_SEND_BUF_SIZE: usize = 1024 * 400; // 400kb
                                                     // 16 MB "sane default" taken from golang http2
const DEFAULT_SETTINGS_MAX_HEADER_LIST_SIZE: u32 = 16 << 20;
/// Default initial stream window size defined in HTTP2 spec.
pub(crate) const SPEC_WINDOW_SIZE: u32 = 65_535;
#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub(crate) adaptive_window: bool,
    pub(crate) initial_conn_window_size: u32,
    pub(crate) initial_stream_window_size: u32,
    pub(crate) max_frame_size: u32,
    pub(crate) enable_connect_protocol: bool,
    pub(crate) max_concurrent_streams: Option<u32>,
    pub(crate) keep_alive_interval: Option<Duration>,
    pub(crate) keep_alive_timeout: Duration,
    pub(crate) max_send_buffer_size: usize,
    pub(crate) max_header_list_size: u32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            adaptive_window: false,
            initial_conn_window_size: DEFAULT_CONN_WINDOW,
            initial_stream_window_size: DEFAULT_STREAM_WINDOW,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            enable_connect_protocol: false,
            max_concurrent_streams: Some(200),
            keep_alive_interval: None,
            keep_alive_timeout: Duration::from_secs(20),
            max_send_buffer_size: DEFAULT_MAX_SEND_BUF_SIZE,
            max_header_list_size: DEFAULT_SETTINGS_MAX_HEADER_LIST_SIZE,
        }
    }
}