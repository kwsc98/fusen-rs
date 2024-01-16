use krpc_core::{client::KrpcClient, common::date_util};
use krpc_macro::krpc_client;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc::{self};

lazy_static! {
    static ref CLI: KrpcClient = KrpcClient::build("http://127.0.0.1:8081".to_string());
}
#[derive(Serialize, Deserialize, Default, Debug)]
struct ReqDto {
    str: String,
}

impl ReqDto {
    fn new() -> Self {
        ReqDto {
            str: "dsdasd".to_string(),
        }
    }
}

struct TestServer;

krpc_client! {
    CLI,
    TestServer,
    "1.0.0",
    async fn do_run1(&self, de : ReqDto) -> ReqDto
    async fn do_run2(&self, de : ReqDto) -> ReqDto
}

#[tokio::main(worker_threads = 500)]
async fn main() {
    let start_time = date_util::get_now_date_time_as_millis();
    let test_server: Arc<TestServer> = Arc::new(TestServer);
    let mut s: (mpsc::Sender<i32>, mpsc::Receiver<i32>) = mpsc::channel(1);
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    drop(s.0);
    let des = s.1.recv().await;
    println!(
        "{:?}",
        date_util::get_now_date_time_as_millis() - start_time
    );
}

async fn do_run(server: Arc<TestServer>, sender: mpsc::Sender<i32>) {
    for idx in 0..200 {
        let s = server.clone();
        let sd = sender.clone();
        tokio::spawn(async move {
            let de = s.do_run1(ReqDto::new()).await;
            println!("{:?}", de);
            drop(sd);
        });
    }
}
