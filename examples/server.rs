use krpc_rust::server::KrpcServer;
use tracing_subscriber::{
    filter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt, Layer,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(fmt::layer().with_filter(filter::LevelFilter::DEBUG))
        .init();
    KrpcServer::build().set_port("8080").run().await;
}
