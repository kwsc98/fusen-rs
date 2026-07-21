use examples::handler::aspect::log::LogAspect;
use examples::handler::aspect::tracing::TraceAspect;
use examples::handler::loadbalance::custom::CustomLoadBalance;
use examples::{DemoServiceClient, DemoServiceV2Client, RequestDto};
use fusen_common::log::LogConfig;
use fusen_common::nacos::NacosConfig;
use fusen_common::nacos::register::NacosRegister;
use fusen_rs::handler::HandlerLoad;
use fusen_rs::{
    client::{ClientOptions, FusenClientContextBuilder},
    contract::WireProtocol,
};
use std::sync::Arc;
use tracing::debug;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_common::log::init_log(
        "fusen-client",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "client={level},examples::handler={level},fusen_rs={level},fusen_common={level}"
                    .to_string(),
            ),
        },
    );
    let mut fusen_contet = FusenClientContextBuilder::new()
        .handler(LogAspect.load())?
        .handler(TraceAspect::default().load())?
        .handler(CustomLoadBalance.load())?
        .build()?;
    debug!("-------------------------使用 Host 直接调用-------------------------");
    let client = DemoServiceClient::init(
        &mut fusen_contet,
        ClientOptions::direct("http://127.0.0.1:8081".parse()?).handlers([
            "CustomLoadBalance",
            "TraceAspect",
            "LogAspect",
        ]),
    )
    .await?;
    let client_v2 = DemoServiceV2Client::init(
        &mut fusen_contet,
        ClientOptions::direct("http://127.0.0.1:8081".parse()?)
            .handlers(["TraceAspect", "LogAspect"]),
    )
    .await?;
    client.sayHelloV4().await?;
    client.divideV2(1, 2).await?;
    client.sayHello("test1".to_owned()).await?;
    client
        .sayHelloV2(RequestDto {
            str: "test2".into(),
        })
        .await?;
    client_v2
        .sayHelloV3(RequestDto {
            str: "test3".into(),
        })
        .await?;

    if let Ok(server_addr) = std::env::var("NACOS_ADDR") {
        debug!("-------------------------使用 Nacos 作为注册中心-------------------------");
        let register = NacosRegister::init_nacos_register(
            "fusen_client",
            Arc::new(NacosConfig {
                server_addr,
                ..Default::default()
            }),
        )?;
        let mut discovery_context = FusenClientContextBuilder::new()
            .handler(LogAspect.load())?
            .handler(TraceAspect::default().load())?
            .register(register)
            .build()?;
        let discovered = DemoServiceClient::init(
            &mut discovery_context,
            ClientOptions::discovery(WireProtocol::Fusen).handlers(["TraceAspect", "LogAspect"]),
        )
        .await?;
        discovered.sayHello("discovery".into()).await?;
        discovered.close().await?;
    }
    client.close().await?;
    client_v2.close().await?;
    Ok(())
}
