use fusen::fusen_macro::{asset, fusen_trait};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ReqDto {
    pub str: String,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct ResDto {
    pub str: String,
}

#[fusen_trait(version = "1.0.0")]
pub trait TestServer {
    async fn do_run1(&self, req1: ReqDto, req2: ReqDto) -> ResDto;

    async fn doRun2(&self, req: ReqDto) -> ResDto;
}

#[fusen_trait(package = "org.apache.dubbo.springboot.demo", version = "1.0.0")]
#[asset(method = POST)]
pub trait DemoService {
    #[asset(path="/sayHello11",method = POST)]
    async fn sayHello(&self, name: String) -> String;

    #[asset(path="/sayHelloV22",method = POST)]
    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;
}
