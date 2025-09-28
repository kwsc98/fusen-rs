use examples::handler::aspect::log::LogAspect;
use examples::handler::aspect::time::TimeAspect;
use examples::{DemoServiceClient, RequestDto};
use fusen_common::date::get_now_date_time_as_millis;
use fusen_common::nacos::NacosConfig;
use fusen_common::nacos::register::NacosRegister;
use fusen_rs::handler::HandlerLoad;
use fusen_rs::{client::FusenClientContextBuilder, fusen_internal_common::protocol::Protocol};
use std::sync::Arc;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() {
    let nacos_register = NacosRegister::init_nacos_register(
        "fusen_client",
        Arc::new(NacosConfig {
            server_addr: "127.0.0.1:8848".to_string(),
            ..Default::default()
        }),
    )
    .unwrap();
    let mut fusen_contet = FusenClientContextBuilder::new()
        .handler(LogAspect.load())
        .handler(TimeAspect.load())
        .register(Box::new(nacos_register))
        .builder();
    let fusen_client = DemoServiceClient::init(&mut fusen_contet, Protocol::Fusen, None)
        .await
        .unwrap();
    let (s, mut r) = mpsc::channel::<i32>(1);
    let start_time = get_now_date_time_as_millis();
    for _ in 0..100 {
        let s_c = s.clone();
        let client_c = fusen_client.clone();
        tokio::spawn(async move {
            for _ in 0..10000 {
                if let Err(error) = client_c
                    .sayHelloV2(RequestDto {
                        str: "test1".to_string(),
                    })
                    .await
                {
                    println!("error : {error:?}")
                }
            }
            drop(s_c);
        });
    }
    drop(s);
    let _result = r.recv().await;
    let time = get_now_date_time_as_millis() - start_time;
    println!("1000000 次请求 耗时 {} 秒 -- {} 毫秒", time / 1000, time);
}
