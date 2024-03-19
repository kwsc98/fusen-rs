use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};

use crate::filter::server::RpcServerFilter;

mod http_handler;
pub mod server;

pub struct StreamHandler {
    pub tcp_stream: TcpStream,
    pub route: &'static RpcServerFilter,
    pub shutdown: broadcast::Receiver<()>,
    pub _shutdown_complete: mpsc::Sender<()>,
}
