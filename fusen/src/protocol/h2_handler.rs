use super::StreamHandler;
use crate::{
    filter::{FusenFilter, RpcServerRoute},
    support::triple::{TripleExceptionWrapper, TripleRequestWrapper, TripleResponseWrapper},
};
use bytes::Bytes;
use h2::server::Builder;
use http::{HeaderMap, HeaderValue, Request, Response};
use fusen_common::{FusenMsg, RpcError};
use prost::Message;
use std::time::Duration;

impl StreamHandler {
    pub async fn run_v2(mut self) {
        let mut connection = get_server_builder()
            .handshake::<_, Bytes>(self.tcp_stream)
            .await
            .unwrap();
        self.filter_list.push(RpcServerRoute::new(self.fusen_server));
        while let Some(result) = connection.accept().await {
            let filter_list = self.filter_list.clone();
            tokio::spawn(async move {
                let (request, mut respond) = result.unwrap();
                let mut trailers = HeaderMap::new();
                match decode_filter(request).await {
                    Ok(mut msg) => {
                        for idx in 0..filter_list.len() {
                            msg = filter_list[idx].call(msg).await.unwrap()
                        }
                        let res = encode_filter(msg).await;
                        let mut send = respond.send_response(res.0, false).unwrap();
                        let _ = send.send_data(res.2, false);
                        trailers.insert("grpc-status", HeaderValue::from_str(&res.1).unwrap());
                        let _ = send.send_trailers(trailers);
                    }
                    Err(err) => {
                        let response: Response<()> = Response::builder()
                            .header("grpc-status", "99")
                            .header("grpc-message", err.to_string())
                            .body(())
                            .unwrap();
                        let _send = respond.send_response(response, true).unwrap();
                    }
                };
            });
        }
    }
}

async fn decode_filter(mut req: Request<h2::RecvStream>) -> crate::Result<FusenMsg> {
    let url = req.uri().path().to_string();
    let data = req.body_mut().data().await.unwrap().unwrap();
    let trip = match TripleRequestWrapper::decode(&data[5..]) {
        Ok(data) => data,
        Err(err) => {
            return Err(Box::new(err));
        }
    };
    let path: Vec<&str> = url.split("/").collect();
    let version = req
        .headers()
        .get("tri-service-version")
        .map(|e| String::from_utf8_lossy(e.as_bytes()).to_string());
    return Ok(FusenMsg::new(
        "unique_identifier".to_string(),
        version,
        path[1].to_string(),
        path[2].to_string(),
        trip.get_req(),
        Result::Err(RpcError::Server("empty".to_string())),
    ));
}

async fn encode_filter(msg: FusenMsg) -> (Response<()>, String, bytes::Bytes) {
    let mut status = "0";
    let res_data = match msg.res {
        Ok(data) => bytes::Bytes::from(TripleResponseWrapper::get_buf(data)),
        Err(err) => bytes::Bytes::from(TripleExceptionWrapper::get_buf(match err {
            RpcError::Client(msg) => {
                status = "90";
                msg
            }
            RpcError::Method(msg) => {
                status = "91";
                msg
            }
            RpcError::Null => {
                status = "92";
                "RpcError::Null".to_string()
            }
            RpcError::Server(msg) => {
                status = "93";
                msg
            }
        })),
    };
    let response: Response<()> = Response::builder()
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(())
        .unwrap();
    return (response, status.to_string(), res_data);
}

fn get_server_builder() -> Builder {
    let mut config: Config = Default::default();
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
    pub(crate) _adaptive_window: bool,
    pub(crate) initial_conn_window_size: u32,
    pub(crate) initial_stream_window_size: u32,
    pub(crate) max_frame_size: u32,
    pub(crate) enable_connect_protocol: bool,
    pub(crate) max_concurrent_streams: Option<u32>,
    pub(crate) _keep_alive_interval: Option<Duration>,
    pub(crate) _keep_alive_timeout: Duration,
    pub(crate) max_send_buffer_size: usize,
    pub(crate) max_header_list_size: u32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            _adaptive_window: false,
            initial_conn_window_size: DEFAULT_CONN_WINDOW,
            initial_stream_window_size: DEFAULT_STREAM_WINDOW,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            enable_connect_protocol: false,
            max_concurrent_streams: Some(200),
            _keep_alive_interval: None,
            _keep_alive_timeout: Duration::from_secs(20),
            max_send_buffer_size: DEFAULT_MAX_SEND_BUF_SIZE,
            max_header_list_size: DEFAULT_SETTINGS_MAX_HEADER_LIST_SIZE,
        }
    }
}
