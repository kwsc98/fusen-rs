use std::{marker::PhantomData, vec};

use crate::{
    filter::{self, KrpcFilter, KrpcRouter},
    support::{TokioExecutor, TokioIo},
};
use bytes::Bytes;
use futures::Future;
use http_body::Body;
use http_body_util::{BodyExt, Full};
use hyper::{server::conn::http2, Request, Response};
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};
use tracing::debug;

use super::KrpcMsg;

pub struct StreamHandler {
    pub tcp_stream: TcpStream,
    pub shutdown: broadcast::Receiver<()>,
    pub _shutdown_complete: mpsc::Sender<()>,
}

impl StreamHandler {
    pub async fn run(mut self) {
        let server = KrpcRouter::new(|req: Request<hyper::body::Incoming>| async move {
            let mut msg = KrpcMsg::new_empty();

            Ok::<_, std::convert::Infallible>(Response::new(Full::<bytes::Bytes>::from(
                "response_body",
            )))
        });
        let hyper_io = TokioIo::new(self.tcp_stream);
        let future = http2::Builder::new(TokioExecutor)
            .initial_stream_window_size(10)
            .initial_connection_window_size(10)
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
