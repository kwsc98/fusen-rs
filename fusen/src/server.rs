use crate::{
    protocol::server::TcpServer,
    register::{Info, Register, RegisterBuilder, Resource},
};
use fusen_common::{server::Protocol, RpcServer};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

pub struct FusenServer {
    protocol: Vec<Protocol>,
    fusen_servers: HashMap<String, Arc<Box<dyn RpcServer>>>,
    register_builder: Vec<RegisterBuilder>,
    register: Vec<Box<dyn Register>>,
}

impl FusenServer {
    pub fn build() -> FusenServer {
        return FusenServer {
            protocol: vec![],
            register_builder: vec![],
            register: vec![],
            fusen_servers: HashMap::new(),
        };
    }
    pub fn add_protocol(mut self, protocol: Protocol) -> FusenServer {
        self.protocol.push(protocol);
        return self;
    }
    pub fn add_register_builder(mut self, register_builder: RegisterBuilder) -> FusenServer {
        self.register_builder.push(register_builder);
        return self;
    }

    pub fn add_fusen_server(mut self, server: Box<dyn RpcServer>) -> FusenServer {
        let info = server.get_info();
        let server_name = info.0.to_string() + "." + &info.1.to_string();
        let mut key = server_name.clone();
        if let Some(version) = info.2 {
            key.push_str(":");
            key.push_str(version);
        }
        self.fusen_servers.insert(key, Arc::new(server));
        return self;
    }

    pub async fn run(&mut self) {
        let tcp_server = TcpServer::init(self.protocol.clone(), self.fusen_servers.clone());
        for register_builder in &self.register_builder {
            let register = register_builder.init(Arc::new(RwLock::new(HashMap::new())));
            if let Ok(port) = register.check(&self.protocol) {
                for server in &self.fusen_servers {
                    let info = server.1.get_info();
                    let server_name = info.0.to_string() + "." + &info.1.to_string();
                    let resource = Resource::Server(Info {
                        server_name,
                        version: info.2.map(|e| e.to_string()),
                        methods: info.3,
                        ip: fusen_common::get_ip(),
                        port: Some(port.clone()),
                    });
                    register.add_resource(resource);
                }
                self.register.push(register);
            }
        }
        let _ = tcp_server.run().await;
    }
}
