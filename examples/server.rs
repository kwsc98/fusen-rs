use krpc_rust::server::KrpcServer;
use tracing_subscriber::{
    fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry().with(fmt::layer()).init();
    KrpcServer::build().set_port("8081").
    
    run().await;
}
