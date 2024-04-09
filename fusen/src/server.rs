use crate::{
    protocol::server::TcpServer,
    register::{Category, Register, RegisterBuilder, Resource},
};
use fusen_common::{
    server::{Protocol, RpcServer, ServerInfo},
    url::UrlConfig,
    MetaData,
};
use std::{collections::HashMap, sync::Arc};

pub struct FusenServer {
    protocol: Vec<Protocol>,
    fusen_servers: HashMap<String, Arc<Box<dyn RpcServer>>>,
    register_config: Vec<Box<dyn UrlConfig>>,
    register: Vec<Box<dyn Register>>,
}

impl FusenServer {
    pub fn build() -> FusenServer {
        return FusenServer {
            protocol: vec![],
            register_config: vec![],
            register: vec![],
            fusen_servers: HashMap::new(),
        };
    }
    pub fn add_protocol(mut self, protocol: Protocol) -> FusenServer {
        self.protocol.push(protocol);
        return self;
    }
    pub fn add_register_builder(mut self, register_config: Box<dyn UrlConfig>) -> FusenServer {
        self.register_config.push(register_config);
        return self;
    }

    pub fn add_fusen_server(mut self, server: Box<dyn RpcServer>) -> FusenServer {
        let info = server.get_info();
        let server_name = info.id.to_string();
        let mut key = server_name.clone();
        if let Some(version) = info.version {
            key.push_str(":");
            key.push_str(&version);
        }
        self.fusen_servers.insert(key, Arc::new(server));
        return self;
    }

    pub async fn run(mut self) {
        let tcp_server = TcpServer::init(self.protocol.clone(), self.fusen_servers.clone());
        let mut shutdown_complete_rx = tcp_server.run().await;
        for register_config in self.register_config {
            let register = RegisterBuilder::new(register_config).unwrap().init();
            if let Ok(port) = register.check(&self.protocol).await {
                for server in &self.fusen_servers {
                    let info: ServerInfo = server.1.get_info();
                    let server_name = info.id.to_string();
                    let resource = Resource {
                        server_name,
                        category: Category::Server,
                        group: info.group,
                        version: info.version,
                        methods: info.methods,
                        ip: fusen_common::net::get_ip(),
                        port: Some(port.clone()),
                        params: MetaData::default().inner,
                    };
                    let _ = register.register(resource).await;
                }
                let _ = self.register.push(register);
            }
        }
        let _ = shutdown_complete_rx.recv().await;
        tracing::info!("fusen server shut");
    }
}
