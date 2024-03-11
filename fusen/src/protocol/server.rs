use fusen_common::server::Protocol;
use fusen_common::RpcServer;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error};

use crate::protocol::StreamHandler;

#[derive(Clone)]
pub struct TcpServer {
    protocol: Vec<Protocol>,
    fusen_servers: HashMap<String, Arc<Box<dyn RpcServer>>>,
}

impl TcpServer {
    pub fn init(
        protocol: Vec<Protocol>,
        fusen_servers: HashMap<String, Arc<Box<dyn RpcServer>>>,
    ) -> Self {
        return TcpServer {
            protocol,
            fusen_servers,
        };
    }
    pub async fn run(self) {
        let (shutdown_complete_tx, mut shutdown_complete_rx) = mpsc::channel(1);
        for protocol in self.protocol {
            tokio::spawn(Self::monitor(
                protocol,
                self.fusen_servers.clone(),
                shutdown_complete_tx.clone(),
            ));
        }
        drop(shutdown_complete_tx);
        let _ = shutdown_complete_rx.recv().await;
        tracing::info!("fusen server shut");
    }

    async fn monitor(
        protocol: Protocol,
        fusen_servers: HashMap<String, Arc<Box<dyn RpcServer>>>,
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
                    tracing::info!("fusen server shut");
                    return Ok(());
                },
                res = listener.accept() => res
            };
            match tcp_stream {
                Ok(stream) => {
                    let stream_handler = StreamHandler {
                        tcp_stream: stream.0,
                        fusen_server: fusen_servers.clone(),
                        shutdown: notify_shutdown.subscribe(),
                        _shutdown_complete: shutdown_complete_tx.clone(),
                    };
                    debug!("socket stream connect, addr: {:?}", stream.1);
                    match &protocol {
                        Protocol::HTTP(_) => tokio::spawn(stream_handler.run_http2()),
                        Protocol::HTTP2(_) => tokio::spawn(stream_handler.run_http1()),
                    };
                }
                Err(err) => error!("tcp connect, err: {:?}", err),
            }
        }
    }
}
