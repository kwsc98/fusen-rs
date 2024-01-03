use krpc_core::server::KrpcServer;
use tracing_subscriber::{
    fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry().with(fmt::layer()).init();
    KrpcServer::build().set_port("8081").run().await;
}

struct TestServer {
    db: Option<String>,
}



impl TestServer {
    
}

#[macro_export]
macro_rules! my_vec {
    // 没带任何参数的 my_vec，我们创建一个空的 vec
    () => {
        std::vec::Vec::new()
    };

    // 处理 my_vec![1, 2, 3, 4]
    ($($el:expr);*) => ({
        let mut v = std::vec::Vec::new();
        $(v.push($el);)*
        v
    });

    // 处理 my_vec![0; 10]
    ($el:expr; $n:expr) => {
        std::vec::from_elem($el, $n)
    }
}
