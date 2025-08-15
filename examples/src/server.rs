use examples::DemoService;
use fusen_rs::{
    error::FusenError, fusen_procedural_macro::fusen_service, server::FusenServerContext,
};

#[derive(Debug, Default)]
struct DemoServiceImpl {
    _db: String,
}

#[fusen_service]
#[asset(path = "/1122")]
impl DemoService for DemoServiceImpl {
    // #[asset(method = GET)]
    async fn sayHello(&self, req: Option<i64>) -> Result<String, FusenError> {
        Ok(format!("Hello {req:?}"))
    }
}

#[tokio::main]
async fn main() {
    let fusen_server =
        FusenServerContext::new(8081).service((Box::new(DemoServiceImpl::default()), vec![]));
    let result = fusen_server.run().await;
    println!("{result:?}");
}
