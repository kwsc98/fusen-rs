use examples::{DemoService, LogAspect, ReqDto, ResDto};
use fusen_rs::filter::ProceedingJoinPoint;
use fusen_rs::fusen_common::config::get_config_by_file;
use fusen_rs::fusen_common::date_util::get_now_date_time_as_millis;
use fusen_rs::fusen_common::logs::LogConfig;
use fusen_rs::fusen_procedural_macro::{asset, handler};
use fusen_rs::handler::aspect::Aspect;
use fusen_rs::handler::HandlerLoad;
use fusen_rs::{fusen_common, FusenApplicationContext};
use fusen_rs::{fusen_common::FusenResult, fusen_procedural_macro::fusen_server};
use tracing::{info, info_span};

struct ServerLogAspect;

#[handler(id = "ServerLogAspect")]
impl Aspect for ServerLogAspect {
    async fn aroud(
        &self,
        join_point: ProceedingJoinPoint,
    ) -> Result<fusen_common::FusenContext, fusen_rs::Error> {
        let start_time = get_now_date_time_as_millis();
        info!("server receive request : {:?}", join_point.get_context());
        let context = join_point.proceed().await;
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

#[fusen_server(id = "org.apache.dubbo.springboot.demo.DemoService")]
impl DemoService for DemoServiceImpl {
    async fn sayHello(&self, req: String) -> FusenResult<String> {
        info!("res : {:?}", req);
        Ok("Hello ".to_owned() + &req)
    }
    #[asset(path="/sayHelloV2-http",method = POST)]
    async fn sayHelloV2(&self, req: ReqDto) -> FusenResult<ResDto> {
        let _span = info_span!("sayHelloV2-http").entered();
        info!("开始处理 sayHelloV2-http");
        info!("接收消息 : {:?}", req);
        let _span2 = info_span!("get_user_info_by_db").entered();
        info!("get_user_info_by_db : selet * from user where id = $1");
        drop(_span2);
        Ok(ResDto::default().str("Hello ".to_owned() + req.get_str() + " V2"))
    }
    #[asset(path="/divide",method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> FusenResult<String> {
        info!("res : a={:?},b={:?}", a, b);
        Ok((a + b).to_string())
    }
}

#[tokio::main]
async fn main() {
    let log_config = LogConfig::default()
        .devmode(Some(true))
        .env_filter(Some(
            "fusen-rs=debug,server=debug,examples=debug".to_owned(),
        ))
        .endpoint(Some("http://127.0.0.1:4317".to_owned()));
    let _log_work = fusen_common::logs::init_log(&log_config, "fusen-server");
    let server = DemoServiceImpl {
        _db: "我是一个DB数据库".to_string(),
    };
    FusenApplicationContext::builder()
        //使用配置文件进行初始化
        .init(get_config_by_file("examples/server-config.yaml").unwrap())
        // .application_name("fusen-server")
        // //初始化Fusen注册中心,同时支持Dubbo3协议与Fusen协议
        // .register(
        //     NacosConfig::default()
        //         .server_addr("127.0.0.1:8848".to_owned())
        //         .to_url()
        //         .unwrap()
        //         .as_str(),
        // )
        // //同时兼容RPC协议与HTTP协议
        // .port(Some(8081))
        // .add_handler_info(HandlerInfo::new(
        //     "org.apache.dubbo.springboot.demo.DemoService".to_owned(),
        //     vec!["ServerLogAspect".to_owned()],
        // ))
        .add_fusen_server(Box::new(server))
        .add_handler(ServerLogAspect.load())
        .add_handler(LogAspect::new("debug").load())
        .build()
        .run()
        .await;
}
