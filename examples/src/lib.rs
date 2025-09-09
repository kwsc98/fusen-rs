use std::time::{SystemTime, UNIX_EPOCH};

use fusen_rs::{
    error::FusenError,
    filter::ProceedingJoinPoint,
    fusen_procedural_macro::{asset, fusen_trait, handler},
    handler::aspect::Aspect,
    protocol::fusen::context::FusenContext,
};
use serde::{Deserialize, Serialize};
// use serde::{Deserialize, Serialize};

// #[derive(Serialize, Deserialize, Default, Debug)]
// pub struct ReqDto {
//     str: String,
// }

// #[derive(Serialize, Deserialize, Default, Debug)]
// pub struct ResDto {
//     str: String,
// }

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ReqDto {
    pub str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ResDto {
    pub str: String,
}

#[fusen_trait]
pub trait DemoService {
    async fn say_hello(&self, name: Option<i64>) -> ();

    async fn say_hellov2(&self, name: Option<String>) -> String;

    async fn say_hellov3(&self, name: Option<String>, name22: i64) -> String;

    #[asset(path = "/name/{name}/age/{age}",method = GET)]
    async fn say_hellov4(&self, name: String, age: String) -> String;

    #[asset(path = "/sayHelloV2-http")]
    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;
}

pub struct LogAspect;

#[handler(id = "LogAspect")]
impl Aspect for LogAspect {
    async fn aroud(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        println!("开始处理 : {:?}", join_point.context.request);
        let context = join_point.proceed().await;
        println!(
            "结束处理 : {:?}",
            context.as_ref().unwrap().response.as_ref().unwrap().body
        );
        context
    }
}

pub struct LogAspectV2;

#[handler(id = "LogAspectV2")]
impl Aspect for LogAspectV2 {
    async fn aroud(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        println!("开始处理时间 : {start_time:?}");
        let context = join_point.proceed().await;
        println!(
            "结束处理时间 : {:?}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()
                - start_time
        );
        context
    }
}
