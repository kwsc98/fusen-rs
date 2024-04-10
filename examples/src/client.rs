use examples::{DemoServiceClient, ReqDto};
use fusen_rs::client::FusenClient;
use fusen_rs::fusen_common;
use fusen_rs::fusen_common::url::UrlConfig;
use fusen_rs::register::nacos::NacosConfig;
use lazy_static::lazy_static;
use tracing::info;

lazy_static! {
    static ref CLI_FUSEN: FusenClient = FusenClient::build(
        NacosConfig::builder()
            .server_addr("127.0.0.1:8848".to_owned())
            .app_name(Some("fusen-client".to_owned()))
            .server_type(fusen_rs::register::Type::Fusen)
            .build()
            .boxed()
    );
    static ref CLI_DUBBO: FusenClient = FusenClient::build(
        NacosConfig::builder()
            .server_addr("127.0.0.1:8848".to_owned())
            .app_name(Some("dubbo-client".to_owned()))
            .server_type(fusen_rs::register::Type::Dubbo)
            .build()
            .boxed()
    );
    static ref CLI_SPRINGCLOUD: FusenClient = FusenClient::build(
        NacosConfig::builder()
            .server_addr("127.0.0.1:8848".to_owned())
            .app_name(Some("springcloud-client".to_owned()))
            .server_type(fusen_rs::register::Type::SpringCloud)
            .build()
            .boxed()
    );
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::logs::init_log();
    //进行Fusen协议调用HTTP2 + JSON
    let client = DemoServiceClient::new(&CLI_FUSEN);
    let res = client
        .sayHelloV2(ReqDto {
            str: "world".to_string(),
        })
        .await;
    info!("rev fusen msg : {:?}", res);

    //进行Dubbo3协议调用HTTP2 + GRPC
    let client = DemoServiceClient::new(&CLI_DUBBO);
    let res = client.sayHello("world".to_string()).await;
    info!("rev dubbo3 msg : {:?}", res);

    //进行SpringCloud协议调用HTTP1 + JSON
    let client = DemoServiceClient::new(&CLI_SPRINGCLOUD);
    let res = client.divideV2(1, 2).await;
    info!("rev springcloud msg : {:?}", res);
}
