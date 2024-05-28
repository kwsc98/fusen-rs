use crate::{handler::HandlerContext, protocol::server::TcpServer};
use fusen_common::server::{Protocol, RpcServer};
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
pub struct FusenServer {
    pub protocol: Vec<Protocol>,
    pub fusen_servers: HashMap<String, &'static dyn RpcServer>,
    pub handler_context: Arc<HandlerContext>,

}

impl FusenServer {
    pub fn new(
        protocol: Vec<Protocol>,
        servers: HashMap<String, Box<dyn RpcServer>>,
        handler_context: Arc<HandlerContext>,
    ) -> FusenServer {
        let mut fusen_servers: HashMap<String, &'static dyn RpcServer> = HashMap::new();
        for (key, server) in servers {
            fusen_servers.insert(key, Box::leak(server));
        }
        FusenServer {
            protocol,
            fusen_servers,
            handler_context
        }
    }

    pub async fn run(&mut self) -> tokio::sync::mpsc::Receiver<()> {
        let tcp_server = TcpServer::init(self.protocol.clone(), self.fusen_servers.clone());
        tcp_server.run(self.handler_context.clone()).await
    }
}
