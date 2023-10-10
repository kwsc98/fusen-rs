use hyper::server::conn::Http;
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};

pub struct StreamHandler {
    pub(crate) tcp_stream: TcpStream,
    pub(crate) shutdown: broadcast::Receiver<()>,
    pub(crate) _shutdown_complete: mpsc::Sender<()>,
}

impl StreamHandler {
    pub async fn run(self) -> crate::Result<()> {
        let future = Http::new()
            .http2_only(true)
            .serve_connection(self.tcp_stream, service);
        
        return Ok(());
    }
}
