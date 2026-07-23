use examples::{
    handler::aspect::{log::LogAspect, tracing::TraceAspect},
    service::{DemoServiceImpl, DemoServiceImplV2},
};
use fusen_common::log::LogConfig;
use fusen_rs::{handler::HandlerLoad, server::FusenServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_common::log::init_log(
        "fusen-host-server",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "host_server={level},examples::handler={level},fusen_rs={level},fusen_common={level}"
                    .to_string(),
            ),
        },
    );
    let bind_addr = "0.0.0.0:8081".parse()?;
    let server = FusenServerBuilder::new(bind_addr)
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
