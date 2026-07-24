use fusen_rs::fusen_trait;
use serde::{Deserialize, Serialize};
pub mod middleware;
pub mod service;

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
    async fn sayHelloV4(&self) -> String;

    async fn sayHello(&self, name: String) -> String;

    #[fusen_rs::asset(path = "/sayHelloV2-http")]
    async fn sayHelloV2(&self, name: RequestDto) -> ResponseDto;

    #[fusen_rs::asset(path = "/divide", method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> String;
}

#[fusen_trait(group = "v1", version = "1.0")]
#[fusen_rs::asset(path = "/dome")]
pub trait DemoServiceV2 {
    #[fusen_rs::asset(path = "/sayHelloV3-http")]
    async fn sayHelloV3(&self, name: RequestDto) -> ResponseDto;
}
