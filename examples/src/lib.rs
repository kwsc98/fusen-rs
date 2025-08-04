use fusen_rs::{error::FusenError, fusen_procedural_macro::{asset, fusen_service}};
// use serde::{Deserialize, Serialize};

// #[derive(Serialize, Deserialize, Default, Debug)]
// pub struct ReqDto {
//     str: String,
// }

// #[derive(Serialize, Deserialize, Default, Debug)]
// pub struct ResDto {
//     str: String,
// }

// #[fusen_service]
pub trait DemoService {
    async fn sayHello(&self, name: String) -> Result<String, FusenError>;

    // #[asset(path = "/sayHelloV2-http", method = POST)]
    // async fn sayHelloV2(&self, name: ReqDto) -> Result<ResDto, FusenError>;

    // #[asset(path = "/divide", method = GET)]
    // async fn divideV2(&self, a: i32, b: Option<String>) -> Result<String, FusenError>;
}
