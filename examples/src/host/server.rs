//! Once-bound direct server example with explicit shutdown.

use examples::{
    DemoServiceServer, DemoServiceV2Server,
    extensions::{
        interceptor::tracing::TracingInterceptor,
        metrics::{LogMetricsRecorder, init_tracing},
    },
    service::{DemoServiceImpl, DemoServiceImplV2},
};
use fusen_rs::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing("host_server=debug,examples::extensions=debug,fusen_rs=debug");
    let running = Server::builder("0.0.0.0:8081")
        .metrics(LogMetricsRecorder)
        .interface(DemoServiceServer::new(DemoServiceImpl).interceptor(TracingInterceptor))
        .interface(DemoServiceV2Server::new(DemoServiceImplV2).interceptor(TracingInterceptor))
        .build()?
        .start()
        .await?;
    println!("host server ready on {}", running.local_addr());
    tokio::signal::ctrl_c().await?;
    running.shutdown().await?;
    Ok(())
}
