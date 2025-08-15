use fusen_rs::{
    error::FusenError,
    fusen_procedural_macro::{asset, fusen_service},
};
// use serde::{Deserialize, Serialize};

// #[derive(Serialize, Deserialize, Default, Debug)]
// pub struct ReqDto {
//     str: String,
// }

// #[derive(Serialize, Deserialize, Default, Debug)]
// pub struct ResDto {
//     str: String,
// }

pub trait DemoService {
    async fn sayHello(&self, name: Option<i64>) -> Result<String, FusenError>;
}
