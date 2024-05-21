pub mod client;
pub mod codec;
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
    server::{Protocol, RpcServer, ServerInfo},
    url::UrlConfig,
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
    protocol: Vec<Protocol>,
    register_config: Vec<Box<dyn UrlConfig>>,
    handlers: Vec<Handler>,
    handler_infos: Vec<HandlerInfo>,
    servers: HashMap<String, Box<dyn RpcServer>>,
}

impl FusenApplicationBuilder {
    pub fn add_register_builder(mut self, register_config: Box<dyn UrlConfig>) -> Self {
        self.register_config.push(register_config);
        self
    }

    pub fn add_protocol(mut self, protocol: Protocol) -> Self {
        self.protocol.push(protocol);
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
            protocol,
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
            let register = Arc::new(RegisterBuilder::new(register_config).unwrap().init());
            let register_type = register.get_type();
            registers.insert(format!("{:?}", register_type), register.clone());
            client.insert(
                format!("{:?}", register_type),
                Arc::new(FusenClient::build(register, handler_context.clone())),
            );
        }
        FusenApplicationContext {
            registers,
            _handler_context: handler_context,
            client,
            server: FusenServer::new(protocol, servers),
        }
    }
}

pub struct FusenApplicationContext {
    registers: HashMap<String, Arc<Box<dyn Register>>>,
    _handler_context: HandlerContext,
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

    pub async fn run(self) {
        let mut shutdown_complete_rx = self.server.run().await;
        for (_id, register) in self.registers {
            if let Ok(port) = register.check(self.server.protocol.clone()).await {
                for server in self.server.fusen_servers.values() {
                    let info: ServerInfo = server.get_info();
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
            }
        }
        let _ = shutdown_complete_rx.recv().await;
        tracing::info!("fusen server shut");
    }
}
