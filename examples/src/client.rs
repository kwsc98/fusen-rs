use std::sync::Arc;

use futures::channel::mpsc::Sender;
use krpc_core::{client::KrpcClient, common::date_util};
use krpc_macro::krpc_client;
use lazy_static::lazy_static;
use tokio::sync::mpsc::{self};

lazy_static! {
    static ref CLI: KrpcClient = KrpcClient::build("http://127.0.0.1:8081".to_string());
}
struct TestServer;
krpc_client! {
    CLI,
    "1.0.0",
    TestServer
    async fn do_run1(&self, de : i32) -> i32
    async fn do_run2(&self, de : i32) -> i32
}

#[tokio::main(worker_threads = 200)]
async fn main() {
    let start_time = date_util::get_now_date_time_as_millis();
    let test_server: Arc<TestServer> = Arc::new(TestServer);
    let mut s: (mpsc::Sender<i32>,mpsc::Receiver<i32>) = mpsc::channel(1);
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    tokio::spawn(do_run(test_server.clone(), s.0.clone()));
    drop(s.0);
    let des = s.1.recv().await;
    println!(
        "{:?}",
        date_util::get_now_date_time_as_millis() - start_time
    );
}

async fn do_run(server: Arc<TestServer>, sender: mpsc::Sender<i32>) {
    for idx in 0..100000 {
        let s = server.clone();
        let sd = sender.clone();
        tokio::spawn(async move {
            let de = s.do_run1(1).await;
            drop(sd);
        });
    }
}
