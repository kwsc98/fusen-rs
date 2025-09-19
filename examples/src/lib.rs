use fusen_rs::fusen_procedural_macro::{asset, fusen_trait};
use serde::{Deserialize, Serialize};
pub mod handler;

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
