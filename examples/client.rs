use async_trait::async_trait;
use bytes::Bytes;
use http::Request;
use http_body_util::{BodyExt as _, Full};
use hyper::client::conn::http1::SendRequest;
use krpc_rust::{
    client::{KrpcClient, KrpcRequest},
    common::date_util,
    support::{TokioExecutor, TokioIo},
};
use std::{thread, time::Duration};
use tokio::{
    io::{self, AsyncWriteExt as _},
    net::TcpStream,
};

#[tokio::main]
async fn main() {
    let start_time = date_util::get_now_date_time_as_millis();
    // tokio::spawn(main1());
    let mut  cli = KrpcClient::build("addr".to_string());
    let k_req
     = KrpcRequest::<String,Option<String>>{ 
        req:"sdd".to_string(),
        res: None
    };
    let de = cli.invoke(k_req).await;
    // loop {
    //     thread::sleep(Duration::from_millis(200));
    //     println!(
    //         "end {:?}",
    //         date_util::get_now_date_time_as_millis() - start_time
    //     );
    // }
}

async fn main1() {
    let url = "http://127.0.0.1:8081".parse::<hyper::Uri>().unwrap();
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
    for _ in 0..10000 {
        let mut send = sender.clone();
        let de = |req| async move {
            let mut res1 = send.send_request(req).await.unwrap();
            println!("res1{:?}", res1);
            while let Some(next) = res1.frame().await {
                let frame = next.unwrap();
                if let Some(chunk) = frame.data_ref() {
                    println!("sdsd1{:?}", chunk);
                }
            }
        };
        tokio::spawn(de(req.clone()));
    }
}
