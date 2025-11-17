use examples::handler::aspect::log::LogAspect;
use examples::handler::aspect::time::TimeAspect;
use examples::handler::aspect::tracing::TraceAspect;
use examples::handler::loadbalance::custom::CustomLoadBalance;
use examples::{DemoServiceClient, DemoServiceV2Client, RequestDto};
use fusen_common::log::LogConfig;
use fusen_common::nacos::NacosConfig;
use fusen_common::nacos::register::NacosRegister;
use fusen_rs::handler::HandlerLoad;
use fusen_rs::{client::FusenClientContextBuilder, fusen_internal_common::protocol::Protocol};
use std::sync::Arc;
use tracing::debug;

#[tokio::main]
async fn main() {
    let _log_work = fusen_common::log::init_log(
        "fusen-client",
        LogConfig {
            level: "debug".to_string(),
            path: Some("log".to_string()),
            endpoint: Some("http://127.0.0.1:4317".to_string()),
            env_filter: Some(
                "client={level},examples::handler={level},fusen_rs={level},fusen_common={level}"
                    .to_string(),
            ),
        },
    );
    let nacos_register = NacosRegister::init_nacos_register(
        "fusen_client",
        Arc::new(NacosConfig {
            server_addr: "127.0.0.1:8848".to_string(),
            ..Default::default()
        }),
    )
    .unwrap();
    let mut fusen_contet = FusenClientContextBuilder::new()
        .handler(LogAspect.load())
        .handler(TimeAspect.load())
        .handler(TraceAspect::default().load())
        .handler(CustomLoadBalance.load())
        // .register(Box::new(nacos_register))
        .builder();
    debug!("-------------------------使用 Host 直接调用-------------------------");
    let client = DemoServiceClient::init(
        &mut fusen_contet,
        Protocol::Host("http://127.0.0.1:8081".to_string()),
        Some(vec![
            "CustomLoadBalance",
            "TraceAspect",
            "LogAspect",
            "TimeAspect",
        ]),
    )
    .await
    .unwrap();
    let client_v2 = DemoServiceV2Client::init(
        &mut fusen_contet,
        Protocol::Host("http://127.0.0.1:8081".to_string()),
        Some(vec!["TraceAspect", "LogAspect"]),
    )
    .await
    .unwrap();
    debug!("{:?}", client.sayHelloV4().await);
    debug!("{:?}", client.divideV2(1, 2).await);
    debug!("{:?}", client.sayHello("test1".to_owned()).await);
    debug!(
        "{:?}",
        client
            .sayHelloV2(RequestDto {
                str: "test2".to_string()
            })
            .await
    );
    debug!(
        "{:?}",
        client_v2
            .sayHelloV3(RequestDto {
                str: "test3".to_string()
            })
            .await
    );
    debug!("-------------------------使用 Nacos 作为注册中心-------------------------");
    //使用 nacos 为注册中心
    let fusen_client = DemoServiceClient::init(
        &mut fusen_contet,
        Protocol::Fusen,
        Some(vec!["TraceAspect", "LogAspect", "TimeAspect"]),
    )
    .await
    .unwrap();
    let fusen_client_v2 = DemoServiceV2Client::init(
        &mut fusen_contet,
        Protocol::Fusen,
        Some(vec!["TraceAspect", "LogAspect"]),
    )
    .await
    .unwrap();
    debug!("{:?}", fusen_client.divideV2(1, 2).await);
    debug!("{:?}", fusen_client.sayHello("test1".to_owned()).await);
    debug!(
        "{:?}",
        fusen_client
            .sayHelloV2(RequestDto {
                str: "test2".to_string()
            })
            .await
    );
    debug!(
        "{:?}",
        fusen_client_v2
            .sayHelloV3(RequestDto {
                str: "test3".to_string()
            })
            .await
    );
}
