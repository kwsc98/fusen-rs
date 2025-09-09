use examples::{DemoService, LogAspect, LogAspectV2, ReqDto, ResDto};
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
    async fn say_hello(&self, _req: Option<i64>) -> Result<(), FusenError> {
        Ok(())
    }

    async fn say_hellov2(&self, req: Option<String>) -> Result<String, FusenError> {
        Ok(format!("Hello {req:?}"))
    }

    async fn say_hellov3(&self, req: Option<String>, ew: i64) -> Result<String, FusenError> {
        Ok(format!("Hello {req:?}  {ew:?}"))
    }

    #[asset(path = "/name/{name}/age/{age}",method = GET)]
    async fn say_hellov4(&self, name: String, age: String) -> Result<String, FusenError> {
        Ok(format!("Hello {name:?} age {age:?}"))
    }

    #[asset(path = "/sayHelloV2-http")]
    async fn sayHelloV2(&self, name: ReqDto) -> Result<ResDto, FusenError> {
        Ok(ResDto {
            str: format!("Hello {:?}", name.str),
        })
    }
}

use jemallocator::Jemalloc;

#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[tokio::main]
async fn main() {
    let nacos = NacosRegister::init(
        NacosConfig {
            application_name: "fusen_service".to_string(),
            server_addr: "127.0.0.1:8848".to_string(),
            ..Default::default()
        },
        fusen_register::support::nacos::Protocol::Fusen,
        None,
    )
    .unwrap();
    let fusen_server = FusenServerContext::new(8081)
        .register(Box::new(nacos))
        .handler(LogAspect.load())
        .handler(LogAspectV2.load())
        .service((
            Box::new(DemoServiceImpl::default()),
            Some(vec!["LogAspectV2", "LogAspect"]),
        ));
    let _result = fusen_server.run().await;
}
