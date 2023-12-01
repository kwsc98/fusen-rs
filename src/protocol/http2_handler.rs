use crate::{
    filter::KrpcRouter,
    support::{TokioExecutor, TokioIo},
};
use http_body_util::{BodyExt, Full};
use hyper::{server::conn::http2, Request, Response};
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};
use tracing::debug;

pub struct StreamHandler {
    pub tcp_stream: TcpStream,
    pub shutdown: broadcast::Receiver<()>,
    pub _shutdown_complete: mpsc::Sender<()>,
}

impl StreamHandler {
    pub async fn run(mut self) {
        let server = KrpcRouter::new(|req: Request<hyper::body::Incoming>| {
            debug!("rev {:?}", req);
            let response_body: Vec<u8> = req.headers().get("1122").unwrap().as_bytes().to_vec();
            async move {
                let mut req_body = req.into_body();
                while let Some(_chunk) = req_body.frame().await {}
                Ok::<_, std::convert::Infallible>(Response::new(Full::<bytes::Bytes>::from(
                    response_body,
                )))
            }
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
