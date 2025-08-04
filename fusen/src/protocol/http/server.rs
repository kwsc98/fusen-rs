use crate::{common::utils::shutdown::Shutdown, server::router::Router};
use hyper_util::rt::TokioExecutor;
use hyper_util::{rt::TokioIo, server::conn::auto::Builder};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::{
    io,
    net::TcpListener,
    sync::{broadcast, mpsc},
};

#[derive(Clone)]
pub struct TcpServer;

impl TcpServer {
    pub async fn run(port: u16, router: Router, mut shutdown: Shutdown) -> io::Result<()> {
        let mut builder = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
        builder.http2().max_concurrent_streams(None);
        builder.http1().keep_alive(true);
        let builder = Arc::new(builder);
        let tcp_listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
        let notify_shutdown: tokio::sync::broadcast::Sender<()> = broadcast::channel(1).0;
        let (sender, mut recv) = mpsc::channel::<()>(1);
        let router = Arc::new(router);
        loop {
            let (tcp_stream, _socketaddr) = tokio::select! {
               stream = tcp_listener.accept() => stream?,
               _ = shutdown.recv() => {
                  drop(notify_shutdown);
                  drop(sender);
                  let _ = recv.recv().await;
                  return Ok(());
               }
            };
            let shutdown = Shutdown::new(notify_shutdown.subscribe());
            let router = router.clone();
            let sender = sender.clone();
            let builder = builder.clone();
            tokio::spawn(async move {
                let handler = HttpStreamHandler {
                    router,
                    builder,
                    stream: tcp_stream,
                    shutdown,
                    _sender: sender,
                };
                handler.run().await;
            });
        }
    }
}

pub struct HttpStreamHandler {
    pub router: Arc<Router>,
    pub builder: Arc<Builder<TokioExecutor>>,
    pub stream: TcpStream,
    pub shutdown: Shutdown,
    pub _sender: mpsc::Sender<()>,
}

impl HttpStreamHandler {
    pub async fn run(self) {
        let HttpStreamHandler {
            router,
            builder,
            stream,
            mut shutdown,
            _sender,
        } = self;
        let hyper_io = TokioIo::new(stream);
        let server = builder.serve_connection_with_upgrades(hyper_io, router);
        let _result = tokio::select! {
            result = server => result,
            _ = shutdown.recv() => Ok(())
        };
    }
}
