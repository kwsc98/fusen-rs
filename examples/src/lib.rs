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

    #[asset(path = "/sayHelloV2-http")]
    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;
}

pub struct LogAspect;

#[handler(id = "LogAspect1")]
impl Aspect for LogAspect {
    async fn aroud(&self, join_point: ProceedingJoinPoint) -> Result<FusenContext, FusenError> {
        let context = join_point.proceed().await;
        context
    }
}
