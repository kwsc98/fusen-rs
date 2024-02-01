use krpc_core::{client::KrpcClient, register::{RegisterBuilder, RegisterType}};
use krpc_macro::krpc_client;
use lazy_static::lazy_static;
use tokio::sync::mpsc;

lazy_static! {
    static ref CLI: KrpcClient = KrpcClient::build(
        RegisterBuilder::new(
            &format!("127.0.0.1:{}", "2181"),
            "default",
            RegisterType::DubboZooKeeper,
        )
    );
}

struct DemoService;

krpc_client! {
   CLI,
   "org.apache.dubbo.springboot.demo",
   DemoService,
   "1.0.0",
   async fn sayHello(&self,name : String) -> Result<String>
} 

#[tokio::main(worker_threads = 512)]
async fn main() {
    krpc_common::init_log();
    let client = DemoService;
    let res = client.sayHello("world".to_string()).await;
    println!("{:?}",res);
    let mut mpsc: (mpsc::Sender<i32>, mpsc::Receiver<i32>) = mpsc::channel(1);
    mpsc.1.recv().await;
}


