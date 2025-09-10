use examples::{DemoService, DemoServiceV2, LogAspect, RequestDto, ResponseDto, TimeAspect};
use fusen_register::support::nacos::{NacosConfig, NacosRegister};
use fusen_rs::{
    error::FusenError,
    fusen_procedural_macro::{asset, fusen_service},
    handler::HandlerLoad,
    server::FusenServerContext,
};

#[derive(Debug, Default)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_service]
impl DemoService for DemoServiceImpl {
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
    async fn divideV2(&self, a: i32, b: i32) -> Result<String, FusenError> {
        Ok(format!("a + b = {}", a + b))
    }
}

#[derive(Debug, Default)]
struct DemoServiceImplV2 {
    _db: String,
}

#[fusen_service]
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
    let nacos = NacosRegister::init(
        NacosConfig {
            application_name: "fusen_service".to_string(),
            server_addr: "127.0.0.1:8848".to_string(),
            ..Default::default()
        },
        None,
    )
    .unwrap();
    let fusen_server = FusenServerContext::new(8081)
        .register(Box::new(nacos))
        .handler(LogAspect.load())
        .handler(TimeAspect.load())
        .service((
            Box::new(DemoServiceImpl::default()),
            Some(vec!["LogAspect", "TimeAspect"]),
        ))
        .service((
            Box::new(DemoServiceImplV2::default()),
            Some(vec!["LogAspect"]),
        ));
    let _result = fusen_server.run().await;
}
