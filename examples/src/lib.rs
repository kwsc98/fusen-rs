use std::time::{SystemTime, UNIX_EPOCH};

use fusen_rs::{
    error::FusenError,
    filter::ProceedingJoinPoint,
    fusen_procedural_macro::{asset, fusen_trait, handler},
    handler::aspect::Aspect,
    protocol::fusen::context::FusenContext,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct RequestDto {
    pub str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ResponseDto {
    pub str: String,
}

#[fusen_trait]
pub trait DemoService {
    async fn sayHello(&self, name: String) -> String;

    #[asset(path = "/sayHelloV2-http")]
    async fn sayHelloV2(&self, name: RequestDto) -> ResponseDto;

    #[asset(path = "/divide", method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> String;
}

#[fusen_trait]
#[asset(path = "/dome")]
pub trait DemoServiceV2 {
    #[asset(path = "/sayHelloV3-http")]
    async fn sayHelloV3(&self, name: RequestDto) -> ResponseDto;
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

pub struct TimeAspect;

#[handler(id = "TimeAspect")]
impl Aspect for TimeAspect {
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
