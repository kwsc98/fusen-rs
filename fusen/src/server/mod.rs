use crate::{
    common::utils::shutdown::Shutdown,
    error::FusenError,
    handler::{Handler, HandlerContext, HandlerInfo},
    protocol::{codec::FusenHttpCodec, http::server::TcpServer},
    server::{
        path::PathCache,
        router::{Router, RouterContext},
        rpc::{RpcServerHandler, RpcService},
    },
};
use fusen_internal_common::{
    protocol::Protocol,
    resource::service::{MethodResource, ServiceResource},
    utils::net::get_network_ip,
};
use fusen_register::Register;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{signal, sync::broadcast};

pub mod path;
pub mod router;
pub mod rpc;

pub struct FusenServerContext {
    port: u16,
    protocol: Protocol,
    registers: Vec<Box<dyn Register>>,
    handler_context: HandlerContext,
    service_handlers: Vec<HandlerInfo>,
    services: HashMap<String, Box<dyn RpcService>>,
}

impl FusenServerContext {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            protocol: Default::default(),
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

    pub fn service(
        mut self,
        (rpc_service, handlers): (Box<dyn RpcService>, Option<Vec<&str>>),
    ) -> Self {
        let service_info = rpc_service.get_service_info();
        if let Some(handlers) = handlers {
            self.service_handlers.push(HandlerInfo {
                service_desc: service_info.service_desc.clone(),
                handlers: handlers.iter().map(|e| e.to_string()).collect(),
            });
        }
        self.services
            .insert(service_info.service_desc.get_tag().to_owned(), rpc_service);
        self
    }

    pub async fn run(mut self) -> Result<(), FusenError> {
        let port = self.port;
        let mut method_infos = vec![];
        let mut service_resources: Vec<Arc<ServiceResource>> = vec![];
        let net_addr = format!("{}:{port}", get_network_ip());
        for rpc_service in self.services.values() {
            let service_info = rpc_service.get_service_info();
            let temp_method_infos = service_info.method_infos.clone();
            service_resources.push(Arc::new(ServiceResource {
                service_id: service_info.service_desc.service_id,
                group: service_info.service_desc.group,
                version: service_info.service_desc.version,
                methods: service_info
                    .method_infos
                    .into_iter()
                    .map(|e| MethodResource {
                        method_name: e.method_name,
                        path: e.path,
                        method: e.method.to_string(),
                    })
                    .collect(),
                addr: net_addr.clone(),
                weight: None,
                metadata: Default::default(),
            }));
            method_infos.append(&mut temp_method_infos.into_iter().map(Arc::new).collect());
        }

        for register in &self.registers {
            for service_resource in &service_resources {
                register
                    .register(service_resource.clone(), self.protocol.clone())
                    .await
                    .map_err(|error| FusenError::Error(Box::new(error)))?;
            }
        }
        for service_handler in self.service_handlers {
            self.handler_context.load_controller(service_handler);
        }
        let router_context = RouterContext {
            http_codec: FusenHttpCodec::default(),
            path_cache: PathCache::build(method_infos).await,
            handler_context: self.handler_context,
            fusen_service_handler: RpcServerHandler::new(self.services),
        };
        let notify_shutdown: tokio::sync::broadcast::Sender<()> = broadcast::channel(1).0;
        let shutdown = Shutdown::new(notify_shutdown.subscribe());
        tokio::spawn(async move {
            let _ = signal::ctrl_c().await;
            for register in self.registers {
                for service_resource in &service_resources {
                    let _ = register
                        .deregister(service_resource.clone(), self.protocol.clone())
                        .await;
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(notify_shutdown);
        });
        let _ = TcpServer::run(
            port,
            Router {
                context: Arc::new(router_context),
            },
            shutdown,
        )
        .await;
        Ok(())
    }
}
