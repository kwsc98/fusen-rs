//! Direct Fusen V1 client example.

use examples::middleware::{
    load_balancer::RandomLoadBalancer,
    log::{LogMetricsRecorder, init_tracing},
    tracing::TracingMiddleware,
};
use examples::{
    DemoService, DemoServiceClient, DemoServiceV2, DemoServiceV2Client, DivideRequest,
    GreetingRequest, HelloRequest, RequestDto,
};
use fusen_rs::{ClientRuntime, RpcRequest};

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
                name: "host".to_owned()
            }))
            .await?
            .into_body()
    );
    println!(
        "{:?}",
        client
            .say_hello_v2(RpcRequest::new(GreetingRequest {
                request: RequestDto {
                    str: "host".to_owned(),
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
                    str: "host-v2".to_owned(),
                }
            }))
            .await?
            .into_body()
    );

    runtime.shutdown().await?;
    Ok(())
}
