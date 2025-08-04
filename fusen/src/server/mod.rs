use crate::{
    common::utils::shutdown::Shutdown,
    error::FusenError,
    handler::{Handler, HandlerContext, HandlerInfo},
    protocol::{codec::FusenHttpCodec, http::server::TcpServer},
    server::{
        path::PathCache,
        router::Router,
        rpc::{RpcServerHandler, RpcService},
    },
};
use fusen_internal_common::resource::service::ServiceResource;
use fusen_register::Register;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{signal, sync::broadcast};

pub mod path;
pub mod router;
pub mod rpc;

pub struct FusenServerContext {
    port: u16,
    registers: Vec<Box<dyn Register>>,
    handler_context: HandlerContext,
    service_handlers: Vec<HandlerInfo>,
    services: HashMap<String, Box<dyn RpcService>>,
}

impl FusenServerContext {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            registers: Default::default(),
            handler_context: HandlerContext::default(),
            service_handlers: Default::default(),
            services: Default::default(),
        }
    }

    pub fn register(mut self, register: Box<dyn Register>) -> Self {
        self.registers.push(register);
        self
    }

    pub fn handler(mut self, handler: Handler) -> Self {
        self.handler_context.load_handler(handler);
        self
    }

    pub fn services(mut self, (rpc_service, handlers): (Box<dyn RpcService>, Vec<String>)) -> Self {
        let service_info = rpc_service.get_service_info();
        self.service_handlers.push(HandlerInfo {
            service_desc: service_info.service_desc.clone(),
            handlers,
        });
        self.services
            .insert(service_info.service_desc.get_tag().to_owned(), rpc_service);
        self
    }

    pub async fn run(mut self) -> Result<(), FusenError> {
        let port = self.port;
        let mut method_infos = vec![];
        for rpc_service in self.services.values() {
            let service_info = rpc_service.get_service_info();
            method_infos.append(
                &mut service_info
                    .method_infos
                    .into_iter()
                    .map(|method_info| Arc::new(method_info))
                    .collect(),
            );
        }
        for service_handler in self.service_handlers {
            self.handler_context.load_controller(service_handler)?;
        }
        let router = Router {
            http_codec: Arc::new(FusenHttpCodec::default()),
            path_cache: Arc::new(PathCache::build(method_infos).await),
            handler_context: Arc::new(self.handler_context),
            fusen_service_handler: Arc::new(RpcServerHandler::new(self.services)),
        };
        let notify_shutdown: tokio::sync::broadcast::Sender<()> = broadcast::channel(1).0;
        let shutdown = Shutdown::new(notify_shutdown.subscribe());
        let service_resources: Vec<Arc<ServiceResource>> = vec![];
        tokio::spawn(async move {
            let _ = signal::ctrl_c().await;
            for register in self.registers {
                for service_resource in &service_resources {
                    let _ = register.deregister(service_resource.clone()).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(notify_shutdown);
        });
        let _ = TcpServer::run(port, router, shutdown).await;
        Ok(())
    }
}
