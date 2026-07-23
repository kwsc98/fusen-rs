use std::sync::Arc;

use examples::handler::aspect::{log::LogAspect, tracing::TraceAspect};
use examples::handler::loadbalance::custom::CustomLoadBalance;
use examples::{DemoServiceClient, DemoServiceV2Client, RequestDto};
use fusen_common::{
    log::LogConfig,
    nacos::{NacosConfig, register::NacosRegister},
};
use fusen_rs::{
    client::{ClientOptions, FusenClientContextBuilder},
    contract::WireProtocol,
    handler::HandlerLoad,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_common::log::init_log(
        "fusen-nacos-client",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "nacos_client={level},examples::handler={level},fusen_rs={level},fusen_common={level}"
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
    )?;
    let mut context = FusenClientContextBuilder::new()
        .register(register)
        .handler(LogAspect.load())?
        .handler(TraceAspect::default().load())?
        .handler(CustomLoadBalance.load())?
        .build()?;
    let client = DemoServiceClient::init(
        &mut context,
        ClientOptions::discovery(WireProtocol::Fusen).handlers([
            "CustomLoadBalance",
            "TraceAspect",
            "LogAspect",
        ]),
    )
    .await?;
    let client_v2 = DemoServiceV2Client::init(
        &mut context,
        ClientOptions::discovery(WireProtocol::Fusen).handlers(["TraceAspect", "LogAspect"]),
    )
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

    client.close().await?;
    client_v2.close().await?;
    Ok(())
}
