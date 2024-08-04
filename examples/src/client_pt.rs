use std::time::Duration;

use examples::{DemoServiceClient, ReqDto};
use fusen_rs::fusen_common::date_util::get_now_date_time_as_millis;
use fusen_rs::fusen_common::register::Type;
use fusen_rs::fusen_common::url::UrlConfig;
use fusen_rs::register::nacos::NacosConfig;
use fusen_rs::{fusen_common, FusenApplicationContext};
use tokio::sync::mpsc;
use tracing::info;

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::logs::init_log();
    let context = FusenApplicationContext::builder()
        .application_name("fusen-client-pt")
        .register_builder(
            NacosConfig::builder()
                .server_addr("127.0.0.1:8848".to_owned())
                .server_type(Type::Fusen)
                .build()
                .boxed()
                .to_url()
                .unwrap()
                .as_str(),
        )
        .build();
    let client = Box::leak(Box::new(DemoServiceClient::new(
        context.client(Type::Fusen).unwrap(),
    )));
    let _ = client
        .sayHelloV2(ReqDto {
            str: "world".to_string(),
        })
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let start_time = get_now_date_time_as_millis();
    let mut m: (mpsc::Sender<i32>, mpsc::Receiver<i32>) = mpsc::channel(1);
    //40 * 100000
    for _ in 0..40 {
        do_run(m.0.clone(), client).await;
    }
    drop(m.0);
    m.1.recv().await;
    info!("{:?}", get_now_date_time_as_millis() - start_time);
}

async fn do_run(send: mpsc::Sender<i32>, client: &'static DemoServiceClient) {
    for _ in 0..100000 {
        let send_clone = send.clone();
        tokio::spawn(async move {
            let res = client
                .sayHelloV2(ReqDto {
                    str: "world".to_string(),
                })
                .await;
            if let Err(err) = res {
                info!("{:?}", err);
            }
            drop(send_clone);
        });
    }
}
