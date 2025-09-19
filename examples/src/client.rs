use examples::handler::log::LogAspect;
use examples::handler::time::TimeAspect;
use examples::{DemoServiceClient, DemoServiceV2Client, RequestDto};
use fusen_register::support::nacos::{NacosConfig, NacosRegister};
use fusen_rs::handler::HandlerLoad;
use fusen_rs::{client::FusenClientContextBuilder, fusen_internal_common::protocol::Protocol};

#[tokio::main]
async fn main() {
    let nacos = NacosRegister::init(
        NacosConfig {
            application_name: "fusen_client".to_string(),
            server_addr: "127.0.0.1:8848".to_string(),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let mut fusen_contet = FusenClientContextBuilder::new()
        .handler(LogAspect.load())
        .handler(TimeAspect.load())
        .register(Box::new(nacos))
        .builder();

    println!("-------------------------使用 Host 直接调用-------------------------");
    let client = DemoServiceClient::init(
        &mut fusen_contet,
        Protocol::Host("http://127.0.0.1:8081".to_string()),
        Some(vec!["LogAspect", "TimeAspect"]),
    )
    .await
    .unwrap();
    let client_v2 = DemoServiceV2Client::init(
        &mut fusen_contet,
        Protocol::Host("http://127.0.0.1:8081".to_string()),
        Some(vec!["LogAspect"]),
    )
    .await
    .unwrap();
    println!("{:?}", client.divideV2(1, 2).await);
    println!("{:?}", client.sayHello("test1".to_owned()).await);
    println!(
        "{:?}",
        client
            .sayHelloV2(RequestDto {
                str: "test2".to_string()
            })
            .await
    );
    println!(
        "{:?}",
        client_v2
            .sayHelloV3(RequestDto {
                str: "test3".to_string()
            })
            .await
    );
    println!("-------------------------使用 Nacos 作为注册中心-------------------------");
    //使用 nacos 为注册中心
    let fusen_client = DemoServiceClient::init(
        &mut fusen_contet,
        Protocol::Fusen,
        Some(vec!["LogAspect", "TimeAspect"]),
    )
    .await
    .unwrap();
    let fusen_client_v2 =
        DemoServiceV2Client::init(&mut fusen_contet, Protocol::Fusen, Some(vec!["LogAspect"]))
            .await
            .unwrap();
    println!("{:?}", fusen_client.divideV2(1, 2).await);
    println!("{:?}", fusen_client.sayHello("test1".to_owned()).await);
    println!(
        "{:?}",
        fusen_client
            .sayHelloV2(RequestDto {
                str: "test2".to_string()
            })
            .await
    );
    println!(
        "{:?}",
        fusen_client_v2
            .sayHelloV3(RequestDto {
                str: "test3".to_string()
            })
            .await
    );
}
