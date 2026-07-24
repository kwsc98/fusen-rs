use examples::middleware::{
    load_balancer::RandomLoadBalancer, log::LogObserver, tracing::TracingMiddleware,
};
use examples::{DemoServiceClient, DemoServiceV2Client, RequestDto};
use fusen_observability::LogConfig;
use fusen_rs::ClientRuntime;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_observability::init_log(
        "fusen-host-client",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "host_client={level},examples::middleware={level},fusen_rs={level}".to_string(),
            ),
        },
    );
    let runtime = ClientRuntime::builder().observer(LogObserver).build()?;
    let client = DemoServiceClient::builder(&runtime)
        .direct("http://127.0.0.1:8081")
        .middleware(TracingMiddleware::default())
        .load_balancer(RandomLoadBalancer)
        .connect()
        .await?;
    let client_v2 = DemoServiceV2Client::builder(&runtime)
        .direct("http://127.0.0.1:8081")
        .middleware(TracingMiddleware::default())
        .connect()
        .await?;

    println!("{}", client.sayHelloV4().await?);
    println!("{}", client.divideV2(6, 2).await?);
    println!("{}", client.sayHello("host".to_owned()).await?);
    println!(
        "{:?}",
        client
            .sayHelloV2(RequestDto {
                str: "host".to_owned(),
            })
            .await?
    );
    println!(
        "{:?}",
        client_v2
            .sayHelloV3(RequestDto {
                str: "host-v2".to_owned(),
            })
            .await?
    );

    runtime.shutdown().await?;
    Ok(())
}
