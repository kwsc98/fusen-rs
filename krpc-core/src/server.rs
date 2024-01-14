use std::{collections::HashMap, sync::Arc};

use krpc_common::RpcServer;
use tokio::sync::Mutex;

use crate::protocol::server::TcpServer;

pub struct KrpcServer {
    port: Option<String>,
    rpc_servers: HashMap<String, Arc<Mutex<Box<dyn RpcServer>>>>,
}

impl KrpcServer {
    pub fn build() -> KrpcServer {
        return KrpcServer {
            port: None,
            rpc_servers: HashMap::new(),
        };
    }

    pub fn set_port(mut self, port: &str) -> KrpcServer {
        let _ = self.port.insert(port.to_string());
        return self;
    }

    pub fn add_rpc_server(mut self, server: Box<dyn RpcServer>) -> KrpcServer {
        self.rpc_servers
            .insert(server.get_info(), Arc::new(Mutex::new(server)));
        return self;
    }

    pub async fn run(&mut self) {
        let port = self.port.clone().unwrap();
        let tcp_server = TcpServer::init(&port[..], self.rpc_servers.clone());
        let _ = tcp_server.run().await;
    }
}
