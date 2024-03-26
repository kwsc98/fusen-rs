use examples::ReqDto;
use examples::TestServerClient;
use fusen::fusen_common;
use fusen::register::nacos::NacosConfig;
use fusen::{
    client::FusenClient,
    register::{RegisterBuilder, RegisterType},
};
use lazy_static::lazy_static;
use tracing::info;
use fusen::fusen_common::url::UrlConfig;


lazy_static! {
    
    static ref CLI: FusenClient =
        FusenClient::build(RegisterBuilder::new(RegisterType::Nacos(
            NacosConfig::new("127.0.0.1:8848", "nacos", "nacos").to_url().unwrap()
        ),));
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    let de = TestServerClient::new(&CLI);
    println!("{:?}", de.get_info());
    fusen_common::logs::init_log();
    let client = de;
    let res = client
        .do_run1(
            ReqDto {
                str: "client say hello 1".to_string(),
            },
            ReqDto {
                str: "client say hello 2".to_string(),
            },
        )
        .await;
    info!("{:?}", res);
    let res = client
        .doRun2(ReqDto {
            str: "client say hello 2".to_string(),
        })
        .await;
    info!("{:?}", res);
}
