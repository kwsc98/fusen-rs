//! Once-bound direct server example with explicit shutdown.

use examples::{
    DemoServiceServer, DemoServiceV2Server,
    middleware::{
        log::{LogMetricsRecorder, init_tracing},
        tracing::TracingMiddleware,
    },
    service::{DemoServiceImpl, DemoServiceImplV2},
};
use fusen_rs::{Server, ServerConfig, contract::ProtocolSet};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing("host_server=debug,examples::middleware=debug,fusen_rs=debug");
    let config = ServerConfig::builder()
        .protocols(ProtocolSet::ALL)
        .build()
        .map_err(std::io::Error::other)?;
    let running = Server::builder("0.0.0.0:8081")
        .config(config)
        .metrics(LogMetricsRecorder)
        .service(DemoServiceServer::new(DemoServiceImpl).middleware(TracingMiddleware))
        .service(DemoServiceV2Server::new(DemoServiceImplV2).middleware(TracingMiddleware))
        .build()?
        .start()
        .await?;
    println!("host server ready on {}", running.local_addr());
    tokio::signal::ctrl_c().await?;
    running.shutdown().await?;
    Ok(())
}
