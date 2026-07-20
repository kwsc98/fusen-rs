use examples::{
    DemoService, DemoServiceV2, RequestDto, ResponseDto,
    handler::aspect::{log::LogAspect, tracing::TraceAspect},
};
use fusen_common::log::LogConfig;
use fusen_rs::{
    error::FusenError,
    fusen_procedural_macro::fusen_service,
    handler::HandlerLoad,
    server::{FusenServerBuilder, ServerConfig},
};

#[derive(Debug, Default)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_service]
impl DemoService for DemoServiceImpl {
    async fn sayHelloV4(&self) -> Result<String, FusenError> {
        Ok("Hello V4".to_string())
    }

    async fn sayHello(&self, name: String) -> Result<String, FusenError> {
        Ok(format!("Hello {name}"))
    }

    async fn sayHelloV2(&self, name: RequestDto) -> Result<ResponseDto, FusenError> {
        Ok(ResponseDto {
            str: format!("HelloV2 {}", name.str),
        })
    }

    async fn divideV2(&self, a: i32, b: i32) -> Result<String, FusenError> {
        Ok(format!("a + b = {}", a + b))
    }
}

#[derive(Debug, Default)]
struct DemoServiceImplV2 {
    _db: String,
}

#[fusen_service]
impl DemoServiceV2 for DemoServiceImplV2 {
    async fn sayHelloV3(&self, name: RequestDto) -> Result<ResponseDto, FusenError> {
        Ok(ResponseDto {
            str: format!("HelloV3 {}", name.str),
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_work = fusen_common::log::init_log(
        "fusen-server",
        LogConfig {
            level: "debug".to_string(),
            path: None,
            endpoint: None,
            env_filter: Some(
                "server={level},examples::handler={level},fusen_rs={level},fusen_common={level}"
                    .to_string(),
            ),
        },
    );
    let bind_addr = "0.0.0.0:8081".parse()?;
    let mut server_config = ServerConfig::new(bind_addr);
    server_config.advertised_base_url = Some("http://127.0.0.1:8081".to_owned());
    let fusen_server = FusenServerBuilder::new(bind_addr)
        .config(server_config)
        .handler(LogAspect.load())?
        .handler(TraceAspect::default().load())?
        .service((
            Box::new(DemoServiceImpl::default()),
            Some(vec!["TraceAspect", "LogAspect"]),
        ))?
        .service((
            Box::new(DemoServiceImplV2::default()),
            Some(vec!["TraceAspect", "LogAspect"]),
        ))?;
    fusen_server.run().await?;
    Ok(())
}
