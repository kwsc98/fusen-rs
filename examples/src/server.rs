use examples::{DemoService, ReqDto, ResDto};
use fusen::fusen_common::url::UrlConfig;
use fusen::fusen_macro::asset;
use fusen::register::nacos::NacosConfig;
use fusen::{
    fusen_common::{self, server::Protocol, FusenResult},
    fusen_macro::fusen_server,
    server::FusenServer,
};
use tracing::info;

#[derive(Clone, Debug)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_server(package = "org.apache.dubbo.springboot.demo", version = "1.0.0")]
#[asset(path = "/DemoServiceHttp")]
impl DemoService for DemoServiceImpl {
    #[asset(path="/sayHello-http",method = POST)]
    async fn sayHello(&self, req: String) -> FusenResult<String> {
        info!("res : {:?}", req);
        return Ok("Hello ".to_owned() + &req);
    }
    async fn sayHelloV2(&self, req: ReqDto) -> FusenResult<ResDto> {
        info!("res : {:?}", req);
        return Ok(ResDto {
            str: "Hello ".to_owned() + &req.str + " V2",
        });
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::logs::init_log();
    let server = DemoServiceImpl {
        _db: "我是一个DB数据库".to_string(),
    };
    FusenServer::build()
        //初始化Fusen注册中心,同时支持Dubbo3协议与Fusen协议
        .add_register_builder(
            NacosConfig::builder()
                .server_addr("127.0.0.1:8848".to_owned())
                .app_name(Some("fusen-service".to_owned()))
                .server_type(fusen::register::Type::Fusen)
                .build()
                .boxed(),
        )
        //初始化SpringCloud注册中心
        .add_register_builder(
            NacosConfig::builder()
                .server_addr("127.0.0.1:8848".to_owned())
                .app_name(Some("springcloud-service".to_owned()))
                .server_type(fusen::register::Type::SpringCloud)
                .build()
                .boxed(),
        )
        .add_protocol(Protocol::HTTP("8081".to_owned()))
        .add_protocol(Protocol::HTTP2("8082".to_owned()))
        .add_fusen_server(Box::new(server))
        .run()
        .await;
}
