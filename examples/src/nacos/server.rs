use std::sync::Arc;

use examples::{
    handler::aspect::{log::LogAspect, tracing::TraceAspect},
    service::{DemoServiceImpl, DemoServiceImplV2},
};
use fusen_common::{
    log::LogConfig,
    nacos::{NacosConfig, register::NacosRegister},
};
use fusen_rs::{
    handler::HandlerLoad,
    server::{FusenServerBuilder, ServerConfig},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_common::log::init_log(
        "fusen-nacos-server",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "nacos_server={level},examples::handler={level},fusen_rs={level},fusen_common={level}"
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
    )?;
    let server = FusenServerBuilder::new(bind_addr)
        .config(server_config)
        .register(register)
        .handler(LogAspect.load())?
        .handler(TraceAspect::default().load())?
        .service((
            Box::new(DemoServiceImpl),
            Some(vec!["TraceAspect", "LogAspect"]),
        ))?
        .service((
            Box::new(DemoServiceImplV2),
            Some(vec!["TraceAspect", "LogAspect"]),
        ))?;
    server.run().await?;
    Ok(())
}
