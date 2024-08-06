use examples::{DemoServiceClient, ReqDto};
use fusen_rs::fusen_common::date_util::get_now_date_time_as_millis;
use fusen_rs::fusen_common::register::Type;
use fusen_rs::fusen_common::url::UrlConfig;
use fusen_rs::fusen_macro::handler;
use fusen_rs::handler::aspect::Aspect;
use fusen_rs::handler::loadbalance::LoadBalance;
use fusen_rs::handler::{HandlerInfo, HandlerLoad};
use fusen_rs::protocol::socket::InvokerAssets;
use fusen_rs::register::nacos::NacosConfig;
use fusen_rs::{fusen_common, FusenApplicationContext};
use rand::prelude::SliceRandom;
use std::sync::Arc;
use tracing::info;

struct CustomLoadBalance;

#[handler(id = "CustomLoadBalance")]
impl LoadBalance for CustomLoadBalance {
    async fn select(
        &self,
        invokers: Vec<Arc<InvokerAssets>>,
    ) -> Result<Arc<InvokerAssets>, fusen_rs::Error> {
        invokers
            .choose(&mut rand::thread_rng())
            .ok_or(fusen_rs::Error::from("not find server : CustomLoadBalance"))
            .cloned()
    }
}

struct ClientLogAspect;

#[handler(id = "ClientLogAspect")]
impl Aspect for ClientLogAspect {
    async fn aroud(
        &self,
        filter: &'static dyn fusen_rs::filter::FusenFilter,
        context: fusen_common::FusenContext,
    ) -> Result<fusen_common::FusenContext, fusen_rs::Error> {
        let start_time = get_now_date_time_as_millis();
        info!("client send request : {:?}", context);
        let context = filter.call(context).await;
        info!(
            "client receive response RT : {:?}ms : {:?}",
            get_now_date_time_as_millis() - start_time,
            context
        );
        context
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 16)]
async fn main() {
    fusen_common::logs::init_log();
    let context = FusenApplicationContext::builder()
        .application_name("fusen-client")
        .register_builder(
            NacosConfig::builder()
                .server_addr("127.0.0.1:8848".to_owned())
                .build()
                .boxed()
                .to_url()
                .unwrap()
                .as_str(),
        )
        .add_handler(CustomLoadBalance.load())
        .add_handler(ClientLogAspect.load())
        //todo! Need to be optimized for configuration
        .add_handler_info(HandlerInfo::new(
            "org.apache.dubbo.springboot.demo.DemoService".to_owned(),
            vec!["CustomLoadBalance".to_owned(), "ClientLogAspect".to_owned()],
        ))
        .build();
    //进行Fusen协议调用HTTP2 + JSON
    // let client = DemoServiceClient::new(Arc::new(
    //     context.client(Type::Host("http://127.0.0.1:8081".to_string())),
    // ));
    let client = DemoServiceClient::new(Arc::new(context.client(Type::Fusen)));
    let res = client
        .sayHelloV2(ReqDto {
            str: "world".to_string(),
        })
        .await;
    info!("rev fusen msg : {:?}", res);
}
