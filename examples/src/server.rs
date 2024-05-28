use examples::{DemoService, ReqDto, ResDto};
use fusen_rs::fusen_common::date_util::get_now_date_time_as_millis;
use fusen_rs::fusen_common::register::Type;
use fusen_rs::fusen_common::url::UrlConfig;
use fusen_rs::fusen_macro::{asset, handler};
use fusen_rs::handler::aspect::Aspect;
use fusen_rs::handler::{HandlerInfo, HandlerLoad};
use fusen_rs::register::nacos::NacosConfig;
use fusen_rs::FusenApplicationContext;
use fusen_rs::{
    fusen_common::{self, server::Protocol, FusenResult},
    fusen_macro::fusen_server,
};
use tracing::info;

struct ServerLogAspect;

#[handler(id = "ServerLogAspect")]
impl Aspect for ServerLogAspect {
    async fn aroud(
        &self,
        filter: &'static dyn fusen_rs::filter::FusenFilter,
        context: fusen_common::FusenContext,
    ) -> Result<fusen_common::FusenContext, fusen_rs::Error> {
        let start_time = get_now_date_time_as_millis();
        info!("server receive request : {:?}", context);
        let context = filter.call(context).await;
        info!(
            "server dispose done RT : {:?}ms : {:?}",
            get_now_date_time_as_millis() - start_time,
            context
        );
        context
    }
}

#[derive(Debug)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_server(package = "org.apache.dubbo.springboot.demo")]
impl DemoService for DemoServiceImpl {
    async fn sayHello(&self, req: String) -> FusenResult<String> {
        info!("res : {:?}", req);
        Ok("Hello ".to_owned() + &req)
    }
    #[asset(path="/sayHelloV2-http",method = POST)]
    async fn sayHelloV2(&self, req: ReqDto) -> FusenResult<ResDto> {
        info!("res : {:?}", req);
        Ok(ResDto {
            str: "Hello ".to_owned() + &req.str + " V2",
        })
    }

    #[asset(path="/divide",method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> FusenResult<String> {
        info!("res : a={:?},b={:?}", a, b);
        Ok((a + b).to_string())
    }
}

#[tokio::main(worker_threads = 512)]
async fn main() {
    fusen_common::logs::init_log();
    let server = DemoServiceImpl {
        _db: "我是一个DB数据库".to_string(),
    };
    //支持多协议，多注册中心的接口暴露
    FusenApplicationContext::builder()
        //初始化Fusen注册中心,同时支持Dubbo3协议与Fusen协议
        .add_register_builder(
            NacosConfig::builder()
                .server_addr("127.0.0.1:8848".to_owned())
                .app_name(Some("fusen-service".to_owned()))
                .server_type(Type::Fusen)
                .build()
                .boxed(),
        )
        //初始化SpringCloud注册中心
        .add_register_builder(
            NacosConfig::builder()
                .server_addr("127.0.0.1:8848".to_owned())
                .app_name(Some("service-provider".to_owned()))
                .server_type(Type::SpringCloud)
                .build()
                .boxed(),
        )
        //同时兼容RPC协议与HTTP协议
        .add_protocol(Protocol::HTTP("8081".to_owned()))
        .add_protocol(Protocol::HTTP2("8082".to_owned()))
        .add_fusen_server(Box::new(server))
        .add_handler(ServerLogAspect.load())
        .add_handler_info(HandlerInfo::new(
            "org.apache.dubbo.springboot.demo.DemoService".to_owned(),
            vec!["ServerLogAspect".to_owned()],
        ))
        .build()
        .run()
        .await;
}
