use std::sync::Arc;
use std::time::Duration;

use examples::{DemoServiceClient, ReqDto};
use fusen_rs::fusen_common::date_util::get_now_date_time_as_millis;
use fusen_rs::fusen_common::register::Type;
use fusen_rs::fusen_common::url::UrlConfig;
use fusen_rs::register::nacos::NacosConfig;
use fusen_rs::{fusen_common, FusenApplicationContext};
use tokio::sync::mpsc;
use tracing::info;

#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() {
    fusen_common::logs::init_log();
    let context = FusenApplicationContext::builder()
        .application_name("fusen-client-pt")
        .register(
            NacosConfig::default()
                .server_addr("127.0.0.1:8848".to_owned())
                .boxed()
                .to_url()
                .unwrap()
                .as_str(),
        )
        .build();
    let client = Box::leak(Box::new(DemoServiceClient::new(Arc::new(
        context.client(Type::Host("127.0.0.1:8081".to_owned())),
    ))));
    let _ = client
        .sayHelloV2(ReqDto {
            str: "world".to_string(),
        })
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let start_time = get_now_date_time_as_millis();
    let mut m: (mpsc::Sender<i32>, mpsc::Receiver<i32>) = mpsc::channel(1);
    for _ in 0..100 {
        tokio::spawn(do_run(m.0.clone(), client));
    }
    drop(m.0);
    m.1.recv().await;
    info!("{:?}", get_now_date_time_as_millis() - start_time);
}

async fn do_run(send: mpsc::Sender<i32>, client: &'static DemoServiceClient) {
    for _ in 0..10000 {
        let res = client
            .sayHelloV2(ReqDto {
                str: "world".to_string(),
            })
            .await;
        if let Err(err) = res {
            info!("{:?}", err);
        }
    }
    drop(send);
}
