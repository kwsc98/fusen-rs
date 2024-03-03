use crate::{
    protocol::server::TcpServer,
    register::{Info, Register, RegisterBuilder, Resource},
};
use fusen_common::RpcServer;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub struct FusenServer {
    port: String,
    rpc_servers: HashMap<String, Arc<Box<dyn RpcServer>>>,
    register: Box<dyn Register>,
}

impl FusenServer {
    pub fn build(register_builder: RegisterBuilder, port: &str) -> FusenServer {
        let map = Arc::new(RwLock::new(HashMap::new()));
        return FusenServer {
            port: port.to_string(),
            register: register_builder.init(map),
            rpc_servers: HashMap::new(),
        };
    }

    pub fn add_rpc_server(mut self, server: Box<dyn RpcServer>) -> FusenServer {
        let info = server.get_info();
        let server_name = info.0.to_string() + "." + &info.1.to_string();
        let mut key = server_name.clone();
        if let Some(version) = info.2 {
            key.push_str(":");
            key.push_str(version);
        }
        self.register.add_resource(Resource::Server(Info {
            server_name,
            version: info.2.map(|e| e.to_string()),
            methods: info.3,
            ip: fusen_common::get_ip(),
            port: Some(self.port.clone()),
        }));
        self.rpc_servers.insert(key, Arc::new(server));
        return self;
    }

    pub async fn run(&mut self) {
        let port = self.port.clone();
        let tcp_server = TcpServer::init(&port[..], self.rpc_servers.clone());
        let _ = tcp_server.run().await;
    }
}
