use examples::{DemoService, ReqDto, ResDto};
use fusen_rs::{
    error::FusenError,
    fusen_procedural_macro::{asset, fusen_service}, server::rpc::RpcService,
};

#[derive(Debug)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_service]
#[asset(path="/1122",method = GET)]
impl DemoService for DemoServiceImpl {
    async fn sayHello(&self, req: String) -> Result<String, FusenError> {
        Ok("Hello ".to_owned() + &req)
    }
    #[asset(path="/sayHelloV2-http",method = POST)]
    async fn sayHelloV2(&self, req: ReqDto) -> Result<ResDto, FusenError> {
        Ok(ResDto::default())
    }
    #[asset(path="/divide",method = GET)]
    async fn divideV2(&self, a: i32, b: Option<String>) -> Result<String, FusenError> {
        Ok((a).to_string())
    }
}