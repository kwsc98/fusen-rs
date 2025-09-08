use examples::{DemoServiceClient, LogAspect, LogAspectV2, ReqDto};
use fusen_register::support::nacos::{NacosConfig, NacosRegister};
use fusen_rs::handler::HandlerLoad;
use fusen_rs::{client::FusenClientContextBuilder, protocol::Protocol};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::{sync::mpsc, time};

#[tokio::main]
async fn main() {
    let nacos = NacosRegister::init(
        NacosConfig {
            application_name: "fusen_client".to_string(),
            server_addr: "127.0.0.1:8848".to_string(),
            ..Default::default()
        },
        fusen_register::support::nacos::Protocol::Fusen,
        None,
    )
    .unwrap();
    let mut fusen_contet = FusenClientContextBuilder::new()
        .handler(LogAspect.load())
        .handler(LogAspectV2.load())
        .register(Box::new(nacos))
        .builder();
    let client = DemoServiceClient::init(
        &mut fusen_contet,
        Protocol::Fusen,
        Some(vec!["LogAspectV2", "LogAspect"]),
    )
    .await
    .unwrap();
    let (s, mut r) = mpsc::channel::<()>(1);
    let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    for _ in 0..1 {
        let client = client.clone();
        let temps = s.clone();
        tokio::spawn(async move {
            for _ in 0..10000 {
                match client
                    .sayHelloV2(ReqDto {
                        str: "world".to_string(),
                    })
                    .await
                {
                    Ok(result) => println!("{result:?}"),
                    Err(error) => println!("{error:?}"),
                }
                time::sleep(Duration::from_secs(1)).await;
            }
            drop(temps);
        });
    }
    drop(s);
    r.recv().await;
    println!(
        "{:?}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            - start_time
    );
    println!("{:?}", client.say_hello(None).await);
    println!("{:?}", client.say_hellov2(Some("dsd".to_string())).await);
    println!("{:?}", client.say_hellov3(Some("dsd".to_string()), 1).await);
}
