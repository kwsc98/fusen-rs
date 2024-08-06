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
use codec::{request_codec::RequestHandler, response_codec::ResponseHandler};
use filter::FusenFilter;
pub use fusen_common;
use fusen_common::{
    register::Type,
    server::{RpcServer, ServerInfo},
    MetaData,
};
pub use fusen_macro;
use handler::{aspect::AspectClientFilter, Handler, HandlerContext};
use register::Register;
use route::client::Route;
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
        let mut handler_context = HandlerContext::default();
        for handler in handlers {
            handler_context.insert(handler);
        }
        for info in handler_infos {
            let _ = handler_context.load_controller(info);
        }
        let register = Arc::new(
            RegisterBuilder::new(register_config)
                .unwrap()
                .init(application_name.clone()),
        );
        let handler_context = Arc::new(handler_context);
        FusenApplicationContext {
            register: register.clone(),
            handler_context: handler_context.clone(),
            client_filter: Box::leak(Box::new(AspectClientFilter::new(
                RequestHandler::new(Arc::new(Default::default())),
                ResponseHandler::new(),
                handler_context.clone(),
                Route::new(register),
            ))),
            server: FusenServer::new(port, servers, handler_context),
        }
    }
}

pub struct FusenApplicationContext {
    register: Arc<Box<dyn Register>>,
    handler_context: Arc<HandlerContext>,
    client_filter: &'static dyn FusenFilter,
    server: FusenServer,
}
impl FusenApplicationContext {
    pub fn builder() -> FusenApplicationBuilder {
        FusenApplicationBuilder::default()
    }

    pub fn client(&self, server_type: Type) -> FusenClient {
        FusenClient::build(
            server_type,
            self.client_filter,
            self.handler_context.clone(),
        )
    }

    pub async fn run(mut self) {
        let port = self.server.port.clone();
        let mut shutdown_complete_rx = self.server.run().await;
        //首先注册server
        let resource = Resource {
            server_name: Default::default(),
            category: Category::Server,
            group: Default::default(),
            version: Default::default(),
            methods: Default::default(),
            host: fusen_common::net::get_ip(),
            port: Some(port.clone()),
            params: MetaData::default().inner,
        };
        let _ = self.register.register(resource).await;
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
                host: fusen_common::net::get_ip(),
                port: Some(port.clone()),
                params: MetaData::default().inner,
            };
            let _ = self.register.register(resource).await;
        }
        let _ = shutdown_complete_rx.recv().await;
        tracing::info!("fusen server shut");
    }
}
