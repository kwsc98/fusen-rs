use crate::{handler::HandlerContext, protocol::server::TcpServer, support::shutdown::Shutdown};
use fusen_common::server::RpcServer;
use fusen_procedural_macro::Data;
use std::{collections::HashMap, sync::Arc};

#[derive(Default, Data)]
pub struct FusenServer {
    port: Option<String>,
    fusen_servers: HashMap<String, &'static dyn RpcServer>,
    handler_context: Arc<HandlerContext>,
}

impl FusenServer {
    pub fn new(
        port: Option<String>,
        servers: HashMap<String, Box<dyn RpcServer>>,
        handler_context: Arc<HandlerContext>,
    ) -> FusenServer {
        let mut fusen_servers: HashMap<String, &'static dyn RpcServer> = HashMap::new();
        for (key, server) in servers {
            fusen_servers.insert(key, Box::leak(server));
        }
        FusenServer {
            port,
            fusen_servers,
            handler_context,
        }
    }

    pub async fn run(&mut self, shutdown: Shutdown) -> tokio::sync::mpsc::Receiver<()> {
        let tcp_server = TcpServer::init(
            self.port.as_ref().expect("not set server port").clone(),
            self.fusen_servers.clone(),
        );
        tcp_server.run(shutdown, self.handler_context.clone()).await
    }
}
