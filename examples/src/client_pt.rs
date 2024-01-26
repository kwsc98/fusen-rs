use krpc_common::date_util::get_now_date_time_as_millis;
use krpc_core::{client::KrpcClient, register::{RegisterBuilder, RegisterType}};
use krpc_macro::krpc_client;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

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

#[derive(Clone)]
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
    let start_time = get_now_date_time_as_millis();
    let client = TestServer;
    let mut m: (mpsc::Sender<i32>, mpsc::Receiver<i32>) = mpsc::channel(1);
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    tokio::spawn(do_run(client.clone(), m.0.clone()));
    drop(m.0);
    let _i32 = m.1.recv().await;
    println!("{:?}",get_now_date_time_as_millis() - start_time);
}

async fn do_run(client : TestServer , sender : mpsc::Sender<i32>) {
    for _idx in 0..100000 {
        let temp_client = client.clone();
        let temp_sender = sender.clone();
        tokio::spawn(async move {
            let res = temp_client.do_run1(ReqDto{str : "client say hello 1".to_string()}).await;
            println!("{:?}",res);
            drop(temp_sender);
        });
    }
}
