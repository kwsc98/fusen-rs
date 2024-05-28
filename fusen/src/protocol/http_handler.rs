use super::StreamHandler;
use crate::{
    route::server::FusenRouter,
    support::{TokioExecutor, TokioIo},
};
use hyper::server::conn::{http1, http2};
use tracing::debug;
impl StreamHandler {
    pub async fn run_http1(mut self) {
        let hyper_io = TokioIo::new(self.tcp_stream);
        let route = FusenRouter::new(self.route, self.http_codec,self.handler_context);
        let future = http1::Builder::new().serve_connection(hyper_io, route);
        let err_info = tokio::select! {
                res = future =>
                    match res {
                        Ok(_) => "client close".to_string(),
                        Err(err) => err.to_string(),
                    }
                 ,
                res2 = self.shutdown.recv() => match res2 {
                    Ok(_) => "http1 shutdown error".to_string(),
                    Err(_) => "http1 server shutdown".to_string(),
                }
        };
        debug!("connect close by {}", err_info);
    }

    pub async fn run_http2(mut self) {
        let hyper_io = TokioIo::new(self.tcp_stream);
        let route = FusenRouter::new(self.route, self.http_codec,self.handler_context);
        let future = http2::Builder::new(TokioExecutor)
            .adaptive_window(true)
            .serve_connection(hyper_io, route);
        let err_info = tokio::select! {
                res = future =>
                    match res {
                        Ok(_) => "client close".to_string(),
                        Err(err) => err.to_string(),
                    }
                 ,
                res2 = self.shutdown.recv() => match res2 {
                    Ok(_) => "http2 shutdown error".to_string(),
                    Err(_) => "http2 server shutdown".to_string(),
                }
        };
        debug!("connect close by {}", err_info);
    }
}
