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
use client::FusenClient;
use codec::{request_codec::RequestHandler, response_codec::ResponseHandler};
use config::FusenApplicationConfig;
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
use std::{collections::HashMap, convert::Infallible, sync::Arc, time::Duration};
use support::shutdown::Shutdown;
use tokio::{
    signal::{self},
    sync::broadcast,
};
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

#[derive(Default)]
pub struct FusenApplicationBuilder {
    port: Option<String>,
    application_name: String,
    register_config: Option<String>,
    handlers: Vec<Handler>,
    handler_infos: Vec<HandlerInfo>,
    servers: HashMap<String, Box<dyn RpcServer>>,
}

impl FusenApplicationBuilder {
    pub fn application_name(mut self, application_name: &str) -> Self {
        application_name.clone_into(&mut self.application_name);
        self
    }

    pub fn port(mut self, port: Option<u16>) -> Self {
        self.port = port.map(|e| e.to_string());
        self
    }

    pub fn register(mut self, register_config: Option<&str>) -> Self {
        self.register_config = register_config.map(|e| e.to_owned());
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

    pub fn init(self, config: FusenApplicationConfig) -> Self {
        let mut builder = self
            .application_name(config.get_application_name())
            .port(*config.get_port())
            .register(config.get_register().as_deref());
        if let Some(handler_infos) = config.get_handler_infos() {
            for handler_info in handler_infos {
                builder = builder.add_handler_info(handler_info.clone());
            }
        }
        builder
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
        let mut register = None;
        if let Some(register_config) = register_config {
            let _ = register.insert(Arc::new(
                RegisterBuilder::new(register_config)
                    .unwrap()
                    .init(application_name.clone()),
            ));
        }
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
    register: Option<Arc<Box<dyn Register>>>,
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
        let (sender, receiver) = broadcast::channel::<()>(1);
        let shutdown = Shutdown::new(receiver);
        let mut shutdown_complete_rx = self.server.run(shutdown).await;
        let mut resources = vec![];
        if let Some(register) = self.register.clone() {
            //首先注册server
            let resource = Resource {
                server_name: Default::default(),
                category: Category::Server,
                group: Default::default(),
                version: Default::default(),
                methods: Default::default(),
                host: fusen_common::net::get_ip(),
                port: port.clone(),
                weight: None,
                params: MetaData::default().inner,
            };
            resources.push(resource.clone());
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
                    host: fusen_common::net::get_ip(),
                    port: port.clone(),
                    weight: None,
                    params: MetaData::default().inner,
                };
                resources.push(resource.clone());
                let _ = register.register(resource).await;
            }
        }
        let register = self.register.clone();
        //如果检测到关机，先注销服务延迟5s后停机
        tokio::spawn(async move {
            tokio::select! {
                _ = signal::ctrl_c() => {
                    if let Some(register) = register {
                        for resource in resources {
                            let _ = register.deregister(resource).await;
                        }
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    drop(sender);
                }
            }
        });
        let _ = shutdown_complete_rx.recv().await;
        tracing::info!("fusen server shut");
    }
}
