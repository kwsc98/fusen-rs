use krpc_macro::rpc_trait;
use serde::{Deserialize, Serialize};

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

    async fn do_run1(&self, req1: ReqDto, req2: ReqDto) -> ResDto;

    async fn doRun2(&self, req: ReqDto) -> ResDto;
    
}
