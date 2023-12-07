use krpc_rust::server::KrpcServer;
use tracing_subscriber::{
    filter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt, Layer,
};

#[tokio::main]
async fn main() {
    KrpcServer::build().set_port("8080").run().await;
}
