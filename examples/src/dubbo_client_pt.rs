use std::time::Duration;

use examples::DemoServiceClient;
use fusen::fusen_common::url::UrlConfig;
use fusen::{
    client::FusenClient,
    fusen_common,
    register::{nacos::NacosConfig, RegisterBuilder, RegisterType},
};
use fusen_common::date_util::get_now_date_time_as_millis;
use lazy_static::lazy_static;
use tokio::sync::mpsc;
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

    let _res = client.sayHello("world".to_string()).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let start_time = get_now_date_time_as_millis();
    let client = client;
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

async fn do_run(client: DemoServiceClient, sender: mpsc::Sender<i32>) {
    for _idx in 0..100000 {
        let temp_client = client.clone();
        let temp_sender = sender.clone();
        tokio::spawn(async move {
            let uuid = fusen_common::logs::get_uuid();
            let res = temp_client.sayHello(uuid.clone()).await;
            info!("req {:?} res {:?}", uuid, res);
            drop(temp_sender);
        });
    }
}
