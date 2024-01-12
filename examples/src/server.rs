use examples::TestInterface;
use hyper::server;
use krpc_common::KrpcMsg;
use krpc_core::server::KrpcServer;
use krpc_macro::krpc_server;
use tracing_subscriber::{
    fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};

krpc_server! {
   TestServer
   async fn do_run1(&self,de : i32) -> i32 {
       return de;
   }
   async fn do_run2(&self,de : i32) -> i32 {
    return de + 1;
}
}

#[tokio::main]
async fn main() {
     let server = TestServer{str : "de".to_string()};
     let mut msg = KrpcMsg::new_empty();
     msg.method_name = "do_run1".to_string();
     msg.data = "2".to_string();
     let de = server.invoke(msg).await;
    println!("{:?}",de);
    // tracing_subscriber::registry().with(fmt::layer()).init();
    // KrpcServer::build().set_port("8081").run().await;
}

struct TestServer {
    str: String,
}
