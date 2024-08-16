use std::sync::Arc;

use hyper_util::{rt::TokioExecutor, server::conn::auto::Builder};
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};

use crate::{
    codec::http_codec::FusenHttpCodec, filter::server::RpcServerFilter, handler::HandlerContext,
};

mod http_handler;
pub mod server;
pub mod socket;

pub struct StreamHandler {
    builder: Arc<Builder<TokioExecutor>>,
    tcp_stream: TcpStream,
    route: &'static RpcServerFilter,
    http_codec: Arc<FusenHttpCodec>,
    handler_context: Arc<HandlerContext>,
    shutdown: broadcast::Receiver<()>,
    _shutdown_complete: mpsc::Sender<()>,
}
