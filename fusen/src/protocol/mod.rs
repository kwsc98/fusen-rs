use std::sync::Arc;

use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};

use crate::{codec::http_codec::FusenHttpCodec, filter::server::RpcServerFilter, handler::HandlerContext};

mod http_handler;
pub mod server;
pub mod socket;

pub struct StreamHandler {
    pub tcp_stream: TcpStream,
    pub route: &'static RpcServerFilter,
    pub http_codec: Arc<FusenHttpCodec>,
    pub handler_context: Arc<HandlerContext>,
    pub shutdown: broadcast::Receiver<()>,
    pub _shutdown_complete: mpsc::Sender<()>,
}
