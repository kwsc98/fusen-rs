use krpc_core::{client::KrpcClient, register::{RegisterBuilder, RegisterType}};
use krpc_macro::krpc_client;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

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
    str: String,
}

#[derive(Serialize, Deserialize, Default,Debug)]
struct ResDto {
    str: String,
}

struct TestServer;

krpc_client! {
   CLI,
   TestServer,
   "1.0.0",
   async fn do_run1(&self,res : ReqDto) -> Result<ResDto>
   async fn do_run2(&self,res : ReqDto) -> Result<ResDto> 
} 

#[tokio::main(worker_threads = 512)]
async fn main() {
    let client = TestServer;
    let res = client.do_run1(ReqDto{str : "client say hello 1".to_string()}).await;
    println!("{:?}",res);
    let res = client.do_run2(ReqDto{str : "client say hello 2".to_string()}).await;
    println!("{:?}",res);
}


