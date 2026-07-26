//! Direct Fusen V1 client example.

use examples::middleware::{
    load_balancer::RandomLoadBalancer,
    log::{LogMetricsRecorder, init_tracing},
    tracing::TracingMiddleware,
};
use examples::{DemoServiceClient, DemoServiceV2Client, RequestDto};
use fusen_rs::ClientRuntime;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing("host_client=debug,examples::middleware=debug,fusen_rs=debug");
    let runtime = ClientRuntime::builder()
        .metrics(LogMetricsRecorder)
        .build()?;
    let client = DemoServiceClient::builder(&runtime)
        .direct("http://127.0.0.1:8081")
        .middleware(TracingMiddleware)
        .load_balancer(RandomLoadBalancer)
        .connect()
        .await?;
    let client_v2 = DemoServiceV2Client::builder(&runtime)
        .direct("http://127.0.0.1:8081")
        .middleware(TracingMiddleware)
        .connect()
        .await?;

    println!("{}", client.say_hello_v4().await?);
    println!("{}", client.divide(6, 2).await?);
    println!("{}", client.say_hello("host".to_owned()).await?);
    println!(
        "{:?}",
        client
            .say_hello_v2(RequestDto {
                str: "host".to_owned(),
            })
            .await?
    );
    println!(
        "{:?}",
        client_v2
            .say_hello_v3(RequestDto {
                str: "host-v2".to_owned(),
            })
            .await?
    );

    runtime.shutdown().await?;
    Ok(())
}
