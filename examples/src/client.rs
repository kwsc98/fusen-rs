
use krpc_core::{common::{KrpcRequest, KrpcResource, date_util}, client::KrpcClient};
use lazy_static::lazy_static;
use tokio::sync::mpsc;

lazy_static! {
    static ref CLI: KrpcClient = KrpcClient::build("http://127.0.0.1:8081".to_string());
    static ref TEST_SERVER: KrpcResource<i32, i32> =
        KrpcResource::new("1.0.0", "TestServer", "do_run1");
}

#[tokio::main(worker_threads = 200)]
async fn main() {
    let start_time = date_util::get_now_date_time_as_millis();
    let k_req = KrpcRequest::new(&TEST_SERVER, 1);
    let mut s: (mpsc::Sender<i32>,mpsc::Receiver<i32>) = mpsc::channel(1);
    for idx in 0..100000 {
        let s1: mpsc::Sender<i32> = s.0.clone();
        let req = k_req.clone();
         tokio::spawn(async move {
            let de = CLI.invoke(req).await;
            // println!("dadasdsda  {:?}",de);
            drop(s1);
         });
    }
    drop(s.0);
    s.1.recv().await;
    println!("{:?}",date_util::get_now_date_time_as_millis() - start_time);
}
