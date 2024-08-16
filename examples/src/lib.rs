use fusen_rs::fusen_procedural_macro::{asset, fusen_trait, Data};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug, Data)]
pub struct ReqDto {
    str: String,
}

#[derive(Serialize, Deserialize, Default, Debug, Data)]
pub struct ResDto {
    str: String,
}

#[fusen_trait(id = "org.apache.dubbo.springboot.demo.DemoService")]
pub trait DemoService {
    async fn sayHello(&self, name: String) -> String;

    #[asset(path = "/sayHelloV2-http", method = POST)]
    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;

    #[asset(path = "/divide", method = GET)]
    async fn divideV2(&self, a: i32, b: i32) -> String;
}
