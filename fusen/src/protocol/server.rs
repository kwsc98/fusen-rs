use crate::codec::http_codec::FusenHttpCodec;
use crate::filter::server::RpcServerFilter;
use crate::handler::HandlerContext;
use crate::protocol::StreamHandler;
use fusen_common::server::Protocol;
use fusen_common::server::RpcServer;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::mpsc::Receiver;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error};

#[derive(Clone)]
pub struct TcpServer {
    protocol: Vec<Protocol>,
    fusen_servers: HashMap<String, &'static dyn RpcServer>,
}

impl TcpServer {
    pub fn init(
        protocol: Vec<Protocol>,
        fusen_servers: HashMap<String, &'static dyn RpcServer>,
    ) -> Self {
        TcpServer {
            protocol,
            fusen_servers,
        }
    }
    pub async fn run(self, handler_context: Arc<HandlerContext>) -> Receiver<()> {
        let (shutdown_complete_tx, shutdown_complete_rx) = mpsc::channel(1);
        let route = Box::leak(Box::new(RpcServerFilter::new(self.fusen_servers)));
        let http_codec = Arc::new(FusenHttpCodec::new(route.get_path_cache()));
        for protocol in self.protocol {
            let http_codec_clone = http_codec.clone();
            let handler_context_clone = handler_context.clone();
            tokio::spawn(Self::monitor(
                protocol,
                route,
                http_codec_clone,
                handler_context_clone,
                shutdown_complete_tx.clone(),
            ));
        }
        drop(shutdown_complete_tx);
        shutdown_complete_rx
    }

    async fn monitor(
        protocol: Protocol,
        route: &'static RpcServerFilter,
        http_codec: Arc<FusenHttpCodec>,
        handler_context: Arc<HandlerContext>,
        shutdown_complete_tx: mpsc::Sender<()>,
    ) -> crate::Result<()> {
        let notify_shutdown = broadcast::channel(1).0;
        let port = match &protocol {
            Protocol::HTTP(port) => port,
            Protocol::HTTP2(port) => port,
        };
        let listener = TcpListener::bind(&format!("0.0.0.0:{}", port)).await?;
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
                        tcp_stream: stream.0,
                        route,
                        http_codec: http_codec.clone(),
                        handler_context: handler_context.clone(),
                        shutdown: notify_shutdown.subscribe(),
                        _shutdown_complete: shutdown_complete_tx.clone(),
                    };
                    debug!("socket stream connect, addr: {:?}", stream.1);
                    match &protocol {
                        Protocol::HTTP(_) => tokio::spawn(stream_handler.run_http1()),
                        Protocol::HTTP2(_) => tokio::spawn(stream_handler.run_http2()),
                    };
                }
                Err(err) => error!("tcp connect, err: {:?}", err),
            }
        }
    }
}
