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

pub struct KrpcMsg {
    pub(crate) unique_identifier: String,
    pub(crate) version: String,
    pub(crate) class_name: String,
    pub(crate) method_name: String,
}

impl KrpcMsg {
    pub fn new_empty() -> KrpcMsg {
        return KrpcMsg {
            unique_identifier: "".to_string(),
            version: "".to_string(),
            class_name: "".to_string(),
            method_name: "".to_string(),
        };
    }
}
