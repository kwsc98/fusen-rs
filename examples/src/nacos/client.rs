//! Nacos-backed discovery client example.

use examples::middleware::{
    load_balancer::RandomLoadBalancer,
    log::{LogMetricsRecorder, init_tracing},
    tracing::TracingMiddleware,
};
use examples::{DemoServiceClient, DemoServiceV2Client, RequestDto};
use fusen_nacos::{NacosConfig, NacosRegistry};
use fusen_rs::ClientRuntime;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing("nacos_client=debug,examples::middleware=debug,fusen_rs=debug,fusen_nacos=debug");
    let config = NacosConfig::builder()
        .server_addr(std::env::var("NACOS_ADDR").unwrap_or_else(|_| "127.0.0.1:8848".to_owned()))
        .build();
    let registry = NacosRegistry::connect("fusen-nacos-client", config).await?;
    let runtime = ClientRuntime::builder()
        .registry(registry)
        .metrics(LogMetricsRecorder)
        .build()?;
    let client = DemoServiceClient::builder(&runtime)
        .discover()
        .middleware(TracingMiddleware)
        .load_balancer(RandomLoadBalancer)
        .connect()
        .await?;
    let client_v2 = DemoServiceV2Client::builder(&runtime)
        .discover()
        .middleware(TracingMiddleware)
        .connect()
        .await?;

    println!("{}", client.say_hello_v4().await?);
    println!("{}", client.divide(6, 2).await?);
    println!("{}", client.say_hello("nacos".to_owned()).await?);
    println!(
        "{:?}",
        client
            .say_hello_v2(RequestDto {
                str: "nacos".to_owned(),
            })
            .await?
    );
    println!(
        "{:?}",
        client_v2
            .say_hello_v3(RequestDto {
                str: "nacos-v2".to_owned(),
            })
            .await?
    );

    runtime.shutdown().await?;
    Ok(())
}
