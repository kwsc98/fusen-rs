use crate::{protocol::server::TcpServer, register::{self, RegisterBuilder}};
use fusen_common::{server::Protocol, RpcServer};
use std::{collections::HashMap, sync::Arc};

pub struct FusenServer {
    protocol: Vec<Protocol>,
    fusen_servers: HashMap<String, Arc<Box<dyn RpcServer>>>,
    register: Vec<RegisterBuilder>,
}

impl FusenServer {
    pub fn build() -> FusenServer {
        return FusenServer {
            protocol: vec![],
            register: vec![],
            fusen_servers: HashMap::new(),
        };
    }
    pub fn add_protocol(&mut self, protocol: Protocol) {
        self.protocol.push(protocol);
    }
    pub fn add_register(&mut self, register_builder: RegisterBuilder) {
        self.register.push(register_builder);
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
        let _ = tcp_server.run().await;
        for register_builder in &self.register {
            let register = register_builder.init();
            if register.check(&self.protocol) {
                  
            }
        }
    }
}
