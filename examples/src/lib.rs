use fusen_macro::rpc_trait;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ReqDto {
    pub str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ResDto {
    pub str: String,
}

#[rpc_trait(package = "com.fusen", version = "1.0.0")]
pub trait TestServer {
    
    async fn do_run1(&self, req1: ReqDto, req2: ReqDto) -> ResDto;

    async fn doRun2(&self, req: ReqDto) -> ResDto;
}

#[rpc_trait(package = "org.apache.dubbo.springboot.demo")]
pub trait DemoService {

    async fn sayHello(&self, name: String) -> String;

    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;
}
