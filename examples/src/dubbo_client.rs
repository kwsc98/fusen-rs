use examples::{DemoServiceRpc, ReqDto};
use fusen::{
    client::FusenClient,
    register::{RegisterBuilder, RegisterType},
};
use lazy_static::lazy_static;
use tracing::info;

lazy_static! {
    static ref CLI: FusenClient = FusenClient::build(RegisterBuilder::new(
        &format!("127.0.0.1:{}", "2181"),
        "default",
        RegisterType::ZooKeeper,
    ));
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::init_log();
    let client = DemoServiceRpc::new(&CLI);
    let res = client.sayHello("world".to_string()).await;
    info!("{:?}", res);
    let res = client
        .sayHelloV2(ReqDto {
            str: "world".to_string(),
        })
        .await;
    info!("{:?}", res);
}
