use std::sync::Arc;

use examples::{
    DemoServiceServer, DemoServiceV2Server,
    middleware::{log::LogObserver, tracing::TracingMiddleware},
    service::{DemoServiceImpl, DemoServiceImplV2},
};
use fusen_nacos::{NacosConfig, NacosRegister};
use fusen_observability::LogConfig;
use fusen_rs::{Server, ServerConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_observability::init_log(
        "fusen-nacos-server",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "nacos_server={level},examples::middleware={level},fusen_rs={level},fusen_nacos={level}"
                    .to_string(),
            ),
        },
    );
    let bind_addr = "0.0.0.0:8081".parse()?;
    let mut server_config = ServerConfig::new(bind_addr);
    server_config.advertised_base_url = Some(
        std::env::var("FUSEN_ADVERTISED_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8081".to_owned()),
    );
    let register = NacosRegister::init_nacos_register(
        "fusen-nacos-server",
        Arc::new(NacosConfig {
            server_addr: std::env::var("NACOS_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8848".to_owned()),
            ..Default::default()
        }),
    )
    .await?;
    let server = Server::bind(bind_addr)
        .config(server_config)
        .registry(register)
        .observer(LogObserver)
        .service(DemoServiceServer::new(DemoServiceImpl).middleware(TracingMiddleware::default()))
        .service(
            DemoServiceV2Server::new(DemoServiceImplV2).middleware(TracingMiddleware::default()),
        );
    server.run().await?;
    Ok(())
}
