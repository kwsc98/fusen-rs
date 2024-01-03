
use krpc_core::{common::{KrpcRequest, KrpcResource}, client::KrpcClient};
use lazy_static::lazy_static;

lazy_static! {
    static ref CLI: KrpcClient = KrpcClient::build("http://127.0.0.1:8081".to_string());
    static ref TEST_SERVER: KrpcResource<String, String> =
        KrpcResource::new("1.0.0", "TestServer", "testMode");
}

#[tokio::main]
async fn main() {
    let k_req = KrpcRequest::new(&TEST_SERVER, "asda".to_string());
    let de = CLI.invoke(k_req).await;
}
