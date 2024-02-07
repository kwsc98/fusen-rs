use std::{collections::HashMap, sync::Arc};

use krpc_common::RpcServer;
use tokio::{
    net::TcpStream,
    sync::{broadcast, mpsc},
};

use crate::filter::Filter;

pub mod server;

mod http2_handler;
mod h2_handler;
mod compression;


pub struct StreamHandler {
    pub tcp_stream: TcpStream,
    pub filter_list: Vec<Filter>,
    pub rpc_server: HashMap<String, Arc<Box<dyn RpcServer>>>,
    pub shutdown: broadcast::Receiver<()>,
    pub _shutdown_complete: mpsc::Sender<()>,
}