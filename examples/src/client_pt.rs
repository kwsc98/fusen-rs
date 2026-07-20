use examples::handler::aspect::log::LogAspect;
use examples::{DemoServiceClient, RequestDto};
use fusen_common::date::get_now_date_time_as_millis;
use fusen_rs::client::{ClientOptions, FusenClientContextBuilder};
use fusen_rs::handler::HandlerLoad;
use tokio::task::JoinSet;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut fusen_contet = FusenClientContextBuilder::new()
        .handler(LogAspect.load())?
        .build()?;
    let fusen_client = DemoServiceClient::init(
        &mut fusen_contet,
        ClientOptions::direct("http://127.0.0.1:8081".parse()?),
    )
    .await?;
    let mut tasks = JoinSet::new();
    let start_time = get_now_date_time_as_millis();
    for _ in 0..100 {
        let client_c = fusen_client.clone();
        tasks.spawn(async move {
            for _ in 0..10000 {
                client_c
                    .sayHelloV2(RequestDto {
                        str: "test1".to_string(),
                    })
                    .await?;
            }
            Ok::<_, fusen_rs::error::FusenError>(())
        });
    }
    while let Some(result) = tasks.join_next().await {
        result??;
    }
    let time = get_now_date_time_as_millis() - start_time;
    println!("1000000 次请求 耗时 {} 秒 -- {} 毫秒", time / 1000, time);
    fusen_client.close().await?;
    Ok(())
}
