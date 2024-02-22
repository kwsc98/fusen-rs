use serde::{Deserialize, Serialize};
use krpc_macro::rpc_trait;

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ReqDto {
    pub str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ResDto {
    pub str: String,
}
#[rpc_trait(package = "com.krpc", version = "1.0.0")]
pub trait TestServer {
    async fn do_run1(&self, res1: ReqDto, res2: ResDto) -> ResDto;
    async fn do_run2(&self, res: ReqDto) -> ResDto;
}