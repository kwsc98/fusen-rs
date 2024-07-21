use super::StreamHandler;
use crate::route::server::FusenRouter;
use hyper_util::rt::TokioIo;
use tracing::debug;
impl StreamHandler {
    pub async fn run_http(mut self) {
        let hyper_io = TokioIo::new(self.tcp_stream);
        let route = FusenRouter::new(self.route, self.http_codec, self.handler_context);
        let conn = self.builder.serve_connection(hyper_io, route);
        let err_info = tokio::select! {
                res = conn =>
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
