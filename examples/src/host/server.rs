use examples::{
    DemoServiceServer, DemoServiceV2Server,
    middleware::{log::LogObserver, tracing::TracingMiddleware},
    service::{DemoServiceImpl, DemoServiceImplV2},
};
use fusen_observability::LogConfig;
use fusen_rs::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_observability::init_log(
        "fusen-host-server",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "host_server={level},examples::middleware={level},fusen_rs={level}".to_string(),
            ),
        },
    );
    let server = Server::bind("0.0.0.0:8081")
        .observer(LogObserver)
        .service(DemoServiceServer::new(DemoServiceImpl).middleware(TracingMiddleware::default()))
        .service(
            DemoServiceV2Server::new(DemoServiceImplV2).middleware(TracingMiddleware::default()),
        );
    server.run().await?;
    Ok(())
}
