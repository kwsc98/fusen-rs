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

#[fusen_trait(package = "org.apache.dubbo.springboot.demo", version = "1.0.0")]
#[asset(path = "/DemoServiceHttp", spring_cloud = "springcloud-service")]
pub trait DemoService {
    #[asset(path="/sayHello-http",method = POST)]
    async fn sayHello(&self, name: String) -> String;

    async fn sayHelloV2(&self, name: ReqDto) -> ResDto;
}
