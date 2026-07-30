//! Nacos-backed discovery client example.

use examples::middleware::{
    load_balancer::RandomLoadBalancer,
    log::{LogMetricsRecorder, init_tracing},
    tracing::TracingMiddleware,
};
use examples::{
    DemoService, DemoServiceClient, DemoServiceV2, DemoServiceV2Client, DivideRequest,
    GreetingRequest, HelloRequest, RequestDto,
};
use fusen_nacos::{NacosConfig, NacosRegistry};
use fusen_rs::{ClientRuntime, RpcRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing("nacos_client=debug,examples::middleware=debug,fusen_rs=debug,fusen_nacos=debug");
    let config = NacosConfig::builder()
        .server_addr(std::env::var("NACOS_ADDR").unwrap_or_else(|_| "127.0.0.1:8848".to_owned()))
        .build()?;
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

    println!(
        "{}",
        client.say_hello_v4(RpcRequest::new(())).await?.into_body()
    );
    println!(
        "{}",
        client
            .divide(RpcRequest::new(DivideRequest { a: 6, b: 2 }))
            .await?
            .into_body()
    );
    println!(
        "{}",
        client
            .say_hello(RpcRequest::new(HelloRequest {
                name: "nacos".to_owned()
            }))
            .await?
            .into_body()
    );
    println!(
        "{:?}",
        client
            .say_hello_v2(RpcRequest::new(GreetingRequest {
                request: RequestDto {
                    str: "nacos".to_owned(),
                }
            }))
            .await?
            .into_body()
    );
    println!(
        "{:?}",
        client_v2
            .say_hello_v3(RpcRequest::new(GreetingRequest {
                request: RequestDto {
                    str: "nacos-v2".to_owned(),
                }
            }))
            .await?
            .into_body()
    );

    runtime.shutdown().await?;
    Ok(())
}
