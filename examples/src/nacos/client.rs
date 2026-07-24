use std::sync::Arc;

use examples::middleware::{
    load_balancer::RandomLoadBalancer, log::LogObserver, tracing::TracingMiddleware,
};
use examples::{DemoServiceClient, DemoServiceV2Client, RequestDto};
use fusen_nacos::{NacosConfig, NacosRegister};
use fusen_observability::LogConfig;
use fusen_rs::ClientRuntime;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_observability::init_log(
        "fusen-nacos-client",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "nacos_client={level},examples::middleware={level},fusen_rs={level},fusen_nacos={level}"
                    .to_string(),
            ),
        },
    );
    let register = NacosRegister::init_nacos_register(
        "fusen-nacos-client",
        Arc::new(NacosConfig {
            server_addr: std::env::var("NACOS_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8848".to_owned()),
            ..Default::default()
        }),
    )
    .await?;
    let runtime = ClientRuntime::builder()
        .registry(register)
        .observer(LogObserver)
        .build()?;
    let client = DemoServiceClient::builder(&runtime)
        .discover()
        .middleware(TracingMiddleware::default())
        .load_balancer(RandomLoadBalancer)
        .connect()
        .await?;
    let client_v2 = DemoServiceV2Client::builder(&runtime)
        .discover()
        .middleware(TracingMiddleware::default())
        .connect()
        .await?;

    println!("{}", client.sayHelloV4().await?);
    println!("{}", client.divideV2(6, 2).await?);
    println!("{}", client.sayHello("nacos".to_owned()).await?);
    println!(
        "{:?}",
        client
            .sayHelloV2(RequestDto {
                str: "nacos".to_owned(),
            })
            .await?
    );
    println!(
        "{:?}",
        client_v2
            .sayHelloV3(RequestDto {
                str: "nacos-v2".to_owned(),
            })
            .await?
    );

    runtime.shutdown().await?;
    Ok(())
}
