use examples::{DemoServiceClient, ReqDto};
use fusen_rs::fusen_common::config::get_config_by_file;
use fusen_rs::fusen_common::date_util::get_now_date_time_as_millis;
use fusen_rs::fusen_common::logs::LogConfig;
use fusen_rs::fusen_common::register::Type;
use fusen_rs::{fusen_common, FusenApplicationContext};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::info;

#[tokio::main]
async fn main() {
    let log_config = LogConfig::default()
    .devmode(Some(true))
    .env_filter(Some(
        "fusen-rs=debug,client=debug,examples=debug".to_owned(),
    ))
    .endpoint(Some("http://127.0.0.1:4317".to_owned()));
let _log_work = fusen_common::logs::init_log(&log_config, "fusen-client-pt");
    let context = FusenApplicationContext::builder()
        .init(get_config_by_file("examples/client-config.yaml").unwrap())
        .build();
    let client = Box::leak(Box::new(DemoServiceClient::new(Arc::new(
        context.client(Type::Fusen),
    ))));
    let _ = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let start_time = get_now_date_time_as_millis();
    let mut m: (mpsc::Sender<i32>, mpsc::Receiver<i32>) = mpsc::channel(1);
    for _ in 0..1 {
        tokio::spawn(do_run(m.0.clone(), client));
    }
    drop(m.0);
    m.1.recv().await;
    info!("{:?}", get_now_date_time_as_millis() - start_time);
}

async fn do_run(send: mpsc::Sender<i32>, client: &'static DemoServiceClient) {
    for _ in 0..1000000 {
        let res = client
            .sayHelloV2(ReqDto::default().str("world".to_string()))
            .await;
        if let Err(err) = res {
            info!("{:?}", err);
        }
    }
    drop(send);
}
