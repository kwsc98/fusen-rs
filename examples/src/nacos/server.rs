//! Nacos-backed registration server example.

use examples::{
    DemoServiceServer, DemoServiceV2Server,
    extensions::{
        interceptor::tracing::TracingInterceptor,
        metrics::{LogMetricsRecorder, init_tracing},
    },
    service::{DemoServiceImpl, DemoServiceImplV2},
};
use fusen_nacos::{NacosConfig, NacosRegistry};
use fusen_rs::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing("nacos_server=debug,examples::extensions=debug,fusen_rs=debug,fusen_nacos=debug");
    let nacos_config = NacosConfig::builder()
        .server_addr(std::env::var("NACOS_ADDR").unwrap_or_else(|_| "127.0.0.1:8848".to_owned()))
        .build()?;
    let registry = NacosRegistry::connect("fusen-nacos-server", nacos_config).await?;
    let advertised_endpoint = std::env::var("FUSEN_ADVERTISED_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8081".to_owned());
    let running = Server::builder("0.0.0.0:8081")
        .advertised_endpoint(advertised_endpoint)
        .registry("nacos", registry)
        .metrics(LogMetricsRecorder)
        .interface(DemoServiceServer::new(DemoServiceImpl).interceptor(TracingInterceptor))
        .interface(DemoServiceV2Server::new(DemoServiceImplV2).interceptor(TracingInterceptor))
        .build()?
        .start()
        .await?;
    println!("Nacos server ready on {}", running.local_addr());
    tokio::signal::ctrl_c().await?;
    running.shutdown().await?;
    Ok(())
}
