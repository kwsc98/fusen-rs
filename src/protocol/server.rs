use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync::{broadcast, mpsc};

use crate::protocol::http2_handler::StreamHandler;

pub struct TcpServer {
    port : String,
    notify_shutdown : broadcast::Sender<()>,
    shutdown_complete_tx: mpsc::Sender<()>,
    shutdown_complete_rx: mpsc::Receiver<()>,
}

impl TcpServer {
    pub fn init(port : &str) -> Self {
        let (shutdown_complete_tx, shutdown_complete_rx) = mpsc::channel(1);
        return TcpServer{
            port : port.to_string(),
            notify_shutdown : broadcast::channel(1).0,
            shutdown_complete_tx,
            shutdown_complete_rx
        }
    }
    async fn run(self) -> crate::Result<()> {
        let listener = TcpListener::bind(&format!("127.0.0.1:{}", self.port)).await?;
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
                    tracing::info!("krpc server shut");
                    return Ok(());
                },
                res = listener.accept() => res
            };
            match tcp_stream {
                Ok(stream) => {
                    let stream_handler = StreamHandler {
                        tcp_stream: stream.0,
                        shutdown: self.notify_shutdown.subscribe(),
                        _shutdown_complete: self.shutdown_complete_tx.clone(),
                    };
                    tracing::debug!("socket stream connect, addr: {:?}",stream.1);
                    tokio::spawn(async move {
                        if let Err(err) = stream_handler.run().await {
                            tracing::error!("stream handler, err: {:?}", err);
                        }else {
                            tracing::debug!("socket stream close");
                        }
                    });
                },
                Err(err) => tracing::error!("tcp connect, err: {:?}", err),
            }
        }

    }
}