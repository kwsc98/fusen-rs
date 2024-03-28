use examples::{DemoServiceClient, ReqDto};
use fusen::fusen_common::url::UrlConfig;
use fusen::{
    client::FusenClient,
    fusen_common,
    register::{nacos::NacosConfig, RegisterBuilder, RegisterType},
};
use lazy_static::lazy_static;
use tracing::info;

lazy_static! {
    static ref CLI: FusenClient = FusenClient::build(RegisterBuilder::new(RegisterType::Nacos(
        NacosConfig::new("127.0.0.1:8848", "nacos", "nacos")
            .to_url()
            .unwrap()
    ),));
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::logs::init_log();
    let client = DemoServiceClient::new(&CLI);
    info!("{:?}", client.get_info());
    let res = client
        .sayHelloV2(ReqDto {
            str: "world".to_string(),
        })
        .await;
    info!("{:?}", res);
}
