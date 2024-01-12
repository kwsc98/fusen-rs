use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};

pub mod server;

mod http2_handler;

pub struct StreamHandler {
    pub tcp_stream: TcpStream,
    pub shutdown: broadcast::Receiver<()>,
    pub _shutdown_complete: mpsc::Sender<()>,
}