use examples::service::{DemoServiceImpl, DemoServiceImplV2};
use fusen_rs::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind_addr: std::net::SocketAddr = std::env::var("PT_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8081".to_owned())
        .parse()?;
    println!("压测服务端监听 {bind_addr}（已关闭逐请求日志和 tracing）");
    Server::bind(bind_addr)
        .service(DemoServiceImpl)
        .service(DemoServiceImplV2)
        .run()
        .await?;
    Ok(())
}
