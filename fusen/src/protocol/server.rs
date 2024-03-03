
use std::collections::HashMap;
use std::sync::Arc;
use fusen_common::RpcServer;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, debug};

use crate::protocol::StreamHandler;

pub struct TcpServer {
    port: String,
    fusen_servers: HashMap<String, Arc<Box<dyn RpcServer>>>,
    notify_shutdown: broadcast::Sender<()>,
    shutdown_complete_tx: mpsc::Sender<()>,
    shutdown_complete_rx: mpsc::Receiver<()>,
}

impl TcpServer {
    pub fn init(port: &str,fusen_servers: HashMap<String, Arc<Box<dyn RpcServer>>>) -> Self {
        let (shutdown_complete_tx, shutdown_complete_rx) = mpsc::channel(1);
        return TcpServer {
            port: port.to_string(),
            fusen_servers,
            notify_shutdown: broadcast::channel(1).0,
            shutdown_complete_tx,
            shutdown_complete_rx,
        };
    }
    pub async fn run(self) -> crate::Result<()> {
        let listener = TcpListener::bind(&format!("0.0.0.0:{}", self.port)).await?;
        loop {
            let tcp_stream = tokio::select! {
                _ = signal::ctrl_c() => {
                    let TcpServer {
                        mut shutdown_complete_rx,
                        shutdown_complete_tx,
                        notify_shutdown,
                        ..
                    } = self;
                    drop(notify_shutdown);
                    drop(shutdown_complete_tx);
                    let _ = shutdown_complete_rx.recv().await;
                    tracing::info!("fusen server shut");
                    return Ok(());
                },
                res = listener.accept() => res
            };
            match tcp_stream {
                Ok(stream) => {
                    let filter_list = vec![];
                    let stream_handler = StreamHandler {
                        tcp_stream: stream.0,
                        filter_list,
                        fusen_server : self.fusen_servers.clone(),
                        shutdown: self.notify_shutdown.subscribe(),
                        _shutdown_complete: self.shutdown_complete_tx.clone(),
                    };
                    debug!("socket stream connect, addr: {:?}", stream.1);
                    tokio::spawn(stream_handler.run_v2());
                }
                Err(err) => error!("tcp connect, err: {:?}", err),
            }
        }
    }
}

