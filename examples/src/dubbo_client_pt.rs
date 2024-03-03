use std::time::Duration;

use fusen_common::date_util::get_now_date_time_as_millis;
use fusen::{
    client::FusenClient, fusen_client, register::{RegisterBuilder, RegisterType}
};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::info;

lazy_static! {
    static ref CLI: FusenClient = FusenClient::build(RegisterBuilder::new(
        &format!("127.0.0.1:{}", "2181"),
        "default",
        RegisterType::ZooKeeper,
    ));
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    name: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
struct ResDto {
    res: String,
}

#[derive(Clone)]
struct DemoService;

fusen_client! {
   CLI,
   "org.apache.dubbo.springboot.demo",
   DemoService,
   None,
   async fn sayHello(&self,name : String) -> Result<String>
   async fn sayHelloV2(&self,name : ReqDto) -> Result<ResDto>
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::init_log();
    let _res = DemoService.sayHello("world".to_string()).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let start_time = get_now_date_time_as_millis();
    let client = DemoService;
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
    info!("{:?}", get_now_date_time_as_millis() - start_time);
}

async fn do_run(client: DemoService, sender: mpsc::Sender<i32>) {
    for _idx in 0..100000 {
        let temp_client = client.clone();
        let temp_sender = sender.clone();
        tokio::spawn(async move {
            let uuid = fusen_common::get_uuid();
            let res = temp_client.sayHello(uuid.clone()).await;
            info!("req {:?} res {:?}", uuid, res);
            drop(temp_sender);
        });
    }
}
