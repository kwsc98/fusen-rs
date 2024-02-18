use krpc_core::{client::KrpcClient, register::{RegisterBuilder, RegisterType}};
use krpc_macro::krpc_client;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use tracing::info;

lazy_static! {
    static ref CLI: KrpcClient = KrpcClient::build(
        RegisterBuilder::new(
            &format!("127.0.0.1:{}", "2181"),
            "default",
            RegisterType::ZooKeeper,
        )
    );
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    name: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct ResDto {
    res : String,
}

struct DemoService;

krpc_client! {
   CLI,
   "org.apache.dubbo.springboot.demo",
   DemoService,
   None,
   async fn sayHello(&self,req : String) -> Result<String>
   async fn sayHelloV2(&self,req : ReqDto) -> Result<ResDto>
} 

#[tokio::main(worker_threads = 512)]
async fn main() {
    krpc_common::init_log();
    let client = DemoService;
    let res = client.sayHello("world".to_string()).await;
    info!("{:?}",res);
    let res = client.sayHelloV2(ReqDto{name:"world".to_string()}).await;
    info!("{:?}",res);
}


