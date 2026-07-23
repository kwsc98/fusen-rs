use examples::handler::aspect::{log::LogAspect, tracing::TraceAspect};
use examples::handler::loadbalance::custom::CustomLoadBalance;
use examples::{DemoServiceClient, DemoServiceV2Client, RequestDto};
use fusen_common::log::LogConfig;
use fusen_rs::{
    client::{ClientOptions, FusenClientContextBuilder},
    handler::HandlerLoad,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_common::log::init_log(
        "fusen-host-client",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "host_client={level},examples::handler={level},fusen_rs={level},fusen_common={level}"
                    .to_string(),
            ),
        },
    );
    let mut context = FusenClientContextBuilder::new()
        .handler(LogAspect.load())?
        .handler(TraceAspect::default().load())?
        .handler(CustomLoadBalance.load())?
        .build()?;
    let client = DemoServiceClient::init(
        &mut context,
        ClientOptions::direct("http://127.0.0.1:8081".parse()?).handlers([
            "CustomLoadBalance",
            "TraceAspect",
            "LogAspect",
        ]),
    )
    .await?;
    let client_v2 = DemoServiceV2Client::init(
        &mut context,
        ClientOptions::direct("http://127.0.0.1:8081".parse()?)
            .handlers(["TraceAspect", "LogAspect"]),
    )
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

    client.close().await?;
    client_v2.close().await?;
    Ok(())
}
