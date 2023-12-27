use krpc_rust::{client::KrpcClient, common::KrpcRequest};
use lazy_static::lazy_static;

lazy_static!{
    static ref CLI : KrpcClient = KrpcClient::build("http://127.0.0.1:8081".to_string());
}



#[tokio::main]
async fn main() {
    let k_req
        = KrpcRequest::<String, String>::new("asda".to_string());
    let de = CLI.invoke(k_req).await;
}
