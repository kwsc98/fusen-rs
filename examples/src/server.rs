use examples::{
    DemoService, DemoServiceV2, RequestDto, ResponseDto,
    handler::aspect::{log::LogAspect, time::TimeAspect, tracing::TraceAspect},
};
use fusen_common::{
    log::LogConfig,
    nacos::{NacosConfig, register::NacosRegister},
};
use fusen_rs::{
    error::FusenError,
    fusen_procedural_macro::{asset, fusen_service},
    handler::HandlerLoad,
    server::FusenServerContext,
};
use std::sync::Arc;
use tracing::instrument;

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

    #[asset(path = "/sayHelloV2-http")]
    async fn sayHelloV2(&self, name: RequestDto) -> Result<ResponseDto, FusenError> {
        Ok(ResponseDto {
            str: format!("HelloV2 {}", name.str),
        })
    }

    #[asset(path = "/divide", method = GET)]
    #[instrument(name = "divideV2", fields(span_type = "HTTP::SERVER::GET"))]
    async fn divideV2(&self, a: i32, b: i32) -> Result<String, FusenError> {
        Ok(format!("a + b = {}", a + b))
    }
}

#[derive(Debug, Default)]
struct DemoServiceImplV2 {
    _db: String,
}

#[fusen_service(group = "v1", version = "1.0")]
#[asset(path = "/dome")]
impl DemoServiceV2 for DemoServiceImplV2 {
    #[asset(path = "/sayHelloV3-http")]
    async fn sayHelloV3(&self, name: RequestDto) -> Result<ResponseDto, FusenError> {
        Ok(ResponseDto {
            str: format!("HelloV3 {}", name.str),
        })
    }
}

#[tokio::main]
async fn main() {
    let _log_work = fusen_common::log::init_log(
        "fusen-server",
        LogConfig {
            level: "debug".to_string(),
            path: Some("log".to_string()),
            endpoint: Some("http://127.0.0.1:4317".to_string()),
            env_filter: Some(
                "server={level},examples::handler={level},fusen_rs={level},fusen_common={level}"
                    .to_string(),
            ),
        },
    );
    let _nacos_register = NacosRegister::init_nacos_register(
        "fusen_server",
        Arc::new(NacosConfig {
            server_addr: "127.0.0.1:8848".to_string(),
            ..Default::default()
        }),
    )
    .unwrap();
    let fusen_server = FusenServerContext::new(8081)
        //开启注册中心
        // .register(Box::new(nacos_register))
        .handler(LogAspect.load())
        .handler(TimeAspect.load())
        .handler(TraceAspect::default().load())
        .service((
            Box::new(DemoServiceImpl::default()),
            Some(vec!["TraceAspect", "LogAspect", "TimeAspect"]),
        ))
        .service((
            Box::new(DemoServiceImplV2::default()),
            Some(vec!["TraceAspect", "LogAspect"]),
        ));
    let _result = fusen_server.run().await;
}
