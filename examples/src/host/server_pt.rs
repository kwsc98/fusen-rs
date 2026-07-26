//! Minimal dual-protocol benchmark server.

use examples::{
    DemoServiceServer, DemoServiceV2Server,
    service::{DemoServiceImpl, DemoServiceImplV2},
};
use fusen_rs::{Server, ServerConfig, contract::ProtocolSet};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind_addr: std::net::SocketAddr = std::env::var("PT_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8081".to_owned())
        .parse()?;
    println!("压测服务端监听 {bind_addr}（已关闭逐请求日志和 tracing）");
    let config = ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .build()
        .map_err(std::io::Error::other)?;
    let running = Server::builder(bind_addr.to_string())
        .config(config)
        .service(DemoServiceServer::new(DemoServiceImpl))
        .service(DemoServiceV2Server::new(DemoServiceImplV2))
        .build()?
        .start()
        .await?;
    tokio::signal::ctrl_c().await?;
    running.shutdown().await?;
    Ok(())
}
