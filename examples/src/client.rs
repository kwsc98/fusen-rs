use examples::{DemoServiceClient, LogAspect, ReqDto};
use fusen_rs::fusen_common::config::get_config_by_file;
use fusen_rs::fusen_common::logs::LogConfig;
use fusen_rs::fusen_common::register::Type;
use fusen_rs::fusen_procedural_macro::handler;
use fusen_rs::handler::loadbalance::LoadBalance;
use fusen_rs::handler::HandlerLoad;
use fusen_rs::protocol::socket::InvokerAssets;
use fusen_rs::register::ResourceInfo;
use fusen_rs::{fusen_common, FusenApplicationContext};
use std::sync::Arc;
use tracing::{info, info_span};

struct CustomLoadBalance;

#[handler(id = "CustomLoadBalance")]
impl LoadBalance for CustomLoadBalance {
    async fn select(
        &self,
        invokers: Arc<ResourceInfo>,
    ) -> Result<Arc<InvokerAssets>, fusen_rs::Error> {
        let _span = info_span!("CustomLoadBalance").or_current();
        invokers
            .select()
            .ok_or("not find server : CustomLoadBalance".into())
    }
}

#[tokio::main]
async fn main() {
    let log_config = LogConfig::default()
        .devmode(Some(true))
        .env_filter(Some(
            "fusen-rs=debug,client=debug,examples=debug".to_owned(),
        ))
        .endpoint(Some("http://127.0.0.1:4317".to_owned()));
    let _log_work = fusen_common::logs::init_log(&log_config, "fusen-client");
    let context = FusenApplicationContext::builder()
        //使用配置文件进行初始化
        .init(get_config_by_file("examples/client-config.yaml").unwrap())
        // .application_name("fusen-client")
        // .register(
        //     NacosConfig::default()
        //         .server_addr("127.0.0.1:8848".to_owned())
        //         .to_url()
        //         .unwrap()
        //         .as_str(),
        // )
        // .add_handler_info(HandlerInfo::new(
        //     "org.apache.dubbo.springboot.demo.DemoService".to_owned(),
        //     vec!["CustomLoadBalance".to_owned(), "ClientLogAspect".to_owned()],
        // ))
        .add_handler(CustomLoadBalance.load())
        .add_handler(LogAspect::new("debug").load())
        .build();
    //直接当HttpClient调用HTTP1 + JSON
    let client = DemoServiceClient::new(Arc::new(
        context.client(Type::Host("127.0.0.1:8082".to_string())),
    ));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev host msg : {:?}", res);
    //通过Fusen进行服务注册与发现，并且进行HTTP2+JSON进行调用
    let client = DemoServiceClient::new(Arc::new(context.client(Type::Fusen)));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev fusen msg : {:?}", res);
    // //通过Dubbo进行服务注册与发现，并且进行HTTP2+Grpc进行调用
    let client = DemoServiceClient::new(Arc::new(context.client(Type::Dubbo)));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev dubbo msg : {:?}", res);
    //通过SpringCloud进行服务注册与发现，并且进行HTTP1+JSON进行调用
    let client = DemoServiceClient::new(Arc::new(context.client(Type::SpringCloud)));
    let res = client
        .sayHelloV2(ReqDto::default().str("world".to_string()))
        .await;
    info!("rev springcloud msg : {:?}", res);
}