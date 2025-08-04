use examples::DemoService;
use fusen_rs::{
    error::FusenError,
    fusen_procedural_macro::{asset, fusen_service},
    server::FusenServerContext,
};

#[derive(Debug, Default)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_service]
#[asset(path="/1122",method = GET)]
impl DemoService for DemoServiceImpl {
    #[asset(method = GET)]
    async fn sayHello(&self, req: String) -> Result<String, FusenError> {
        Ok("Hello ".to_owned() + &req)
    }
}

#[tokio::main]
async fn main() {
    let fusen_server =
        FusenServerContext::new(8081).services((Box::new(DemoServiceImpl::default()), vec![]));
    let result = fusen_server.run().await;
    println!("{:?}", result);
}
