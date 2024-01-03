use crate::protocol::server::TcpServer;

pub struct KrpcServer {
    port: Option<String>,

}

impl KrpcServer {
    
    pub fn build() -> KrpcServer {
        return KrpcServer { port: None };
    }

    pub fn set_port(mut self, port: &str) -> KrpcServer {
        let _ = self.port.insert(port.to_string());
        return self;
    }

    pub async fn run(&mut self) {
        let port = self.port.clone().unwrap();
        let tcp_server = TcpServer::init(&port[..]);
        let _ = tcp_server.run().await;
    }
}


