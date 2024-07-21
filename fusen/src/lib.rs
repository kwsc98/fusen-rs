pub mod client;
pub mod codec;
pub mod config;
pub mod filter;
pub mod handler;
pub mod protocol;
pub mod register;
pub mod route;
pub mod server;
pub mod support;
use crate::{
    handler::HandlerInfo,
    register::{Category, RegisterBuilder, Resource},
};
use bytes::Buf;
use client::FusenClient;
pub use fusen_common;
use fusen_common::{
    register::Type,
    server::{RpcServer, ServerInfo},
    MetaData,
};
pub use fusen_macro;
use handler::{Handler, HandlerContext};
use register::Register;
use server::FusenServer;
use std::{collections::HashMap, convert::Infallible, sync::Arc};
pub type Error = fusen_common::Error;
pub type Result<T> = fusen_common::Result<T>;
pub type FusenFuture<T> = fusen_common::FusenFuture<T>;
pub type HttpBody = futures_util::stream::Iter<
    std::vec::IntoIter<std::result::Result<http_body::Frame<bytes::Bytes>, Infallible>>,
>;
pub type BoxBody<D, E> = http_body_util::combinators::BoxBody<D, E>;

pub type StreamBody<D, E> = http_body_util::StreamBody<
    futures_util::stream::Iter<std::vec::IntoIter<std::result::Result<http_body::Frame<D>, E>>>,
>;

fn get_empty_body<D, E>() -> BoxBody<D, E>
where
    D: Buf + 'static,
{
    BoxBody::default()
}

#[derive(Default)]
pub struct FusenApplicationBuilder {
    port: String,
    application_name: String,
    register_config: String,
    handlers: Vec<Handler>,
    handler_infos: Vec<HandlerInfo>,
    servers: HashMap<String, Box<dyn RpcServer>>,
}

impl FusenApplicationBuilder {
    pub fn application_name(mut self, application_name: &str) -> Self {
        self.application_name = application_name.to_owned();
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = port.to_string();
        self
    }

    pub fn register_builder(mut self, register_config: &str) -> Self {
        self.register_config = register_config.to_owned();
        self
    }

    pub fn add_fusen_server(mut self, server: Box<dyn RpcServer>) -> Self {
        let info = server.get_info();
        let server_name = info.id.to_string();
        let mut key = server_name.clone();
        if let Some(version) = info.version {
            key.push(':');
            key.push_str(&version);
        }
        self.servers.insert(key, server);
        self
    }

    pub fn add_handler(mut self, handler: Handler) -> Self {
        self.handlers.push(handler);
        self
    }
    pub fn add_handler_info(mut self, info: HandlerInfo) -> Self {
        self.handler_infos.push(info);
        self
    }

    pub fn build(self) -> FusenApplicationContext {
        let FusenApplicationBuilder {
            application_name,
            port,
            register_config,
            handlers,
            handler_infos,
            servers,
        } = self;
        let mut registers: HashMap<String, Arc<Box<dyn Register>>> = HashMap::new();
        let mut client: HashMap<String, Arc<FusenClient>> = HashMap::new();
        let mut handler_context = HandlerContext::default();
        for handler in handlers {
            handler_context.insert(handler);
        }
        for info in handler_infos {
            let _ = handler_context.load_controller(info);
        }
        for register_config in register_config {
            let register = Arc::new(
                RegisterBuilder::new(register_config)
                    .unwrap()
                    .init(application_name.clone()),
            );
            let register_type = register.get_type();
            registers.insert(format!("{:?}", register_type), register.clone());
            client.insert(
                format!("{:?}", register_type),
                Arc::new(FusenClient::build(
                    register,
                    Arc::new(handler_context.clone()),
                )),
            );
        }
        let handler_context = Arc::new(handler_context);
        FusenApplicationContext {
            registers,
            _handler_context: handler_context.clone(),
            client,
            server: FusenServer::new(port, servers, handler_context),
        }
    }
}

pub struct FusenApplicationContext {
    registers: HashMap<String, Arc<Box<dyn Register>>>,
    _handler_context: Arc<HandlerContext>,
    client: HashMap<String, Arc<FusenClient>>,
    server: FusenServer,
}
impl FusenApplicationContext {
    pub fn builder() -> FusenApplicationBuilder {
        FusenApplicationBuilder::default()
    }

    pub fn client(&self, ty: Type) -> Option<Arc<FusenClient>> {
        self.client.get(&format!("{:?}", ty)).cloned()
    }

    pub async fn run(mut self) {
        let port = self.server.port.clone();
        let mut shutdown_complete_rx = self.server.run().await;
        for (_id, register) in self.registers {
            //首先注册server
            let resource = Resource {
                server_name: Default::default(),
                category: Category::Server,
                group: Default::default(),
                version: Default::default(),
                methods: Default::default(),
                ip: fusen_common::net::get_ip(),
                port: Some(port.clone()),
                params: MetaData::default().inner,
            };
            let _ = register.register(resource).await;
            //再注册service
            for server in self.server.fusen_servers.values() {
                let info: ServerInfo = server.get_info();
                let server_name = info.id.to_string();
                let resource = Resource {
                    server_name,
                    category: Category::Service,
                    group: info.group,
                    version: info.version,
                    methods: info.methods,
                    ip: fusen_common::net::get_ip(),
                    port: Some(port.clone()),
                    params: MetaData::default().inner,
                };
                let _ = register.register(resource).await;
            }
        }
        let _ = shutdown_complete_rx.recv().await;
        tracing::info!("fusen server shut");
    }
}
