use std::{thread, time::Duration};

use http::Request;
use http_body_util::{BodyExt as _, Full};
use krpc_rust::{
    common::date_util,
    support::{TokioExecutor, TokioIo},
};
use tokio::{
    io::{self, AsyncWriteExt as _},
    net::TcpStream,
    sync::broadcast,
};
use tracing::debug;
use tracing_subscriber::{
    filter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt, Layer,
};

#[tokio::main(flavor = "multi_thread", worker_threads = 50)]
async fn main() {
    let start_time = date_util::get_now_date_time_as_millis();
    tokio::spawn(main1());
    tokio::spawn(main1());

    loop {
        thread::sleep(Duration::from_millis(100));
        println!(
            "end {:?}",
            date_util::get_now_date_time_as_millis() - start_time
        );
    }
}

async fn main1() {
    let url = "http://127.0.0.1:8080".parse::<hyper::Uri>().unwrap();
    let host = url.host().expect("uri has no host");
    let port = url.port_u16().unwrap_or(80);
    let addr = format!("{}:{}", host, port);
    let stream = TcpStream::connect(addr).await.unwrap();
    let stream = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http2::handshake(TokioExecutor, stream)
        .await
        .unwrap();
    tokio::spawn(async move {
        if let Err(err) = conn.await {
            let mut stdout = io::stdout();
            stdout
                .write_all(format!("Connection failed: {:?}", err).as_bytes())
                .await
                .unwrap();
            stdout.flush().await.unwrap();
        }
    });
    let req = Request::builder()
        .header("unique_identifier", "unique_identifier")
        .header("version", "version")
        .header("class_name", "class_name")
        .header("method_name", "method_name")
        .body(Full::<bytes::Bytes>::from("ds"))
        .unwrap();
    let start_time = date_util::get_now_date_time_as_millis();
    let mut re: (broadcast::Sender<i32>, broadcast::Receiver<_>) = broadcast::channel(1);

    for _ in 0..10000 {
        let send = re.0.clone();
        let mut sender1 = sender.clone();
        let de = |req, send| async move {
            let mut res1 = sender1.send_request(req).await.unwrap();
            println!("res1{:?}", res1);
            while let Some(next) = res1.frame().await {
                let frame = next.unwrap();
                if let Some(chunk) = frame.data_ref() {
                    println!("sdsd1{:?}", chunk);
                }
            }
        };
        tokio::spawn(de(req.clone(), send.clone()));
    }
    let req = Request::builder()
        .header("unique_identifier", "unique_identifier")
        .header("version", "version")
        .header("class_name", "class_name")
        .header("method_name", "method_name")
        .body(Full::<bytes::Bytes>::from("ds"))
        .unwrap();
    let mut res2 = sender.send_request(req).await.unwrap();
    debug!("res1{:?}", res2);
    while let Some(next) = res2.frame().await {
        let frame = next.unwrap();
        if let Some(chunk) = frame.data_ref() {
            debug!("sdsd2{:?}", chunk);
        }
    }
    let mut re: (broadcast::Sender<i32>, broadcast::Receiver<_>) = broadcast::channel(1);
    drop(re.0);
    re.1.recv().await;
    debug!(
        "end   {:?}",
        date_util::get_now_date_time_as_millis() - start_time
    );
}
