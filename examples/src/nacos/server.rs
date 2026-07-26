//! Nacos-backed registration server example.

use examples::{
    DemoServiceServer, DemoServiceV2Server,
    middleware::{
        log::{LogMetricsRecorder, init_tracing},
        tracing::TracingMiddleware,
    },
    service::{DemoServiceImpl, DemoServiceImplV2},
};
use fusen_nacos::{NacosConfig, NacosRegistry};
use fusen_rs::{Server, ServerConfig, contract::ProtocolSet};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing("nacos_server=debug,examples::middleware=debug,fusen_rs=debug,fusen_nacos=debug");
    let server_config = ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .build()
        .map_err(std::io::Error::other)?;
    let nacos_config = NacosConfig::builder()
        .server_addr(std::env::var("NACOS_ADDR").unwrap_or_else(|_| "127.0.0.1:8848".to_owned()))
        .build();
    let registry = NacosRegistry::connect("fusen-nacos-server", nacos_config).await?;
    let advertised_endpoint = std::env::var("FUSEN_ADVERTISED_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8081".to_owned());
    let running = Server::builder("0.0.0.0:8081")
        .config(server_config)
        .advertised_endpoint(advertised_endpoint)
        .registry("nacos", registry)
        .metrics(LogMetricsRecorder)
        .service(DemoServiceServer::new(DemoServiceImpl).middleware(TracingMiddleware))
        .service(DemoServiceV2Server::new(DemoServiceImplV2).middleware(TracingMiddleware))
        .build()?
        .start()
        .await?;
    println!("Nacos server ready on {}", running.local_addr());
    tokio::signal::ctrl_c().await?;
    running.shutdown().await?;
    Ok(())
}
