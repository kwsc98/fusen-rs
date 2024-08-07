use crate::codec::http_codec::FusenHttpCodec;
use crate::filter::server::RpcServerFilter;
use crate::handler::HandlerContext;
use crate::protocol::StreamHandler;
use fusen_common::server::RpcServer;
use hyper_util::rt::TokioExecutor;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::mpsc::Receiver;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error};

#[derive(Clone)]
pub struct TcpServer {
    port: String,
    fusen_servers: HashMap<String, &'static dyn RpcServer>,
}

impl TcpServer {
    pub fn init(port: String, fusen_servers: HashMap<String, &'static dyn RpcServer>) -> Self {
        TcpServer {
            port,
            fusen_servers,
        }
    }
    pub async fn run(self, handler_context: Arc<HandlerContext>) -> Receiver<()> {
        let (shutdown_complete_tx, shutdown_complete_rx) = mpsc::channel(1);
        let route = Box::leak(Box::new(RpcServerFilter::new(self.fusen_servers)));
        let http_codec = Arc::new(FusenHttpCodec::new(route.get_path_cache()));
        let port = self.port;
        tokio::spawn(Self::monitor(
            port,
            route,
            http_codec,
            handler_context,
            shutdown_complete_tx.clone(),
        ));
        drop(shutdown_complete_tx);
        shutdown_complete_rx
    }

    async fn monitor(
        port: String,
        route: &'static RpcServerFilter,
        http_codec: Arc<FusenHttpCodec>,
        handler_context: Arc<HandlerContext>,
        shutdown_complete_tx: mpsc::Sender<()>,
    ) -> crate::Result<()> {
        let notify_shutdown = broadcast::channel(1).0;
        let listener = TcpListener::bind(&format!("0.0.0.0:{}", port)).await?;
        let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
        builder.http2().max_concurrent_streams(None);
        builder.http1().keep_alive(true);
        let builder = Arc::new(builder);
        loop {
            let tcp_stream = tokio::select! {
                _ = signal::ctrl_c() => {
                    drop(notify_shutdown);
                    drop(shutdown_complete_tx);
                    return Ok(());
                },
                res = listener.accept() => res
            };
            match tcp_stream {
                Ok(stream) => {
                    let stream_handler = StreamHandler {
                        builder: builder.clone(),
                        tcp_stream: stream.0,
                        route,
                        http_codec: http_codec.clone(),
                        handler_context: handler_context.clone(),
                        shutdown: notify_shutdown.subscribe(),
                        _shutdown_complete: shutdown_complete_tx.clone(),
                    };
                    debug!("socket stream connect, addr: {:?}", stream.1);
                    tokio::spawn(stream_handler.run_http());
                }
                Err(err) => error!("tcp connect, err: {:?}", err),
            }
        }
    }
}
